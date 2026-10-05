//! Explicit observer for [`ToTokens`] interpolations.
//!
//! This module provides [`Observer`], a trait for instrumenting the token
//! output of individual interpolation fragments inside a [`quote!`]
//! invocation. Each interpolation is assigned a unique [`SourceId`] and the
//! range of top-level tokens it produces is recorded explicitly - the range
//! is measured, not guessed from the final string.
//!
//! The observed quote macro [`quote_observe!`] calls [`ToTokens::to_tokens`]
//! exactly once per interpolation, records a receipt even when the fragment
//! produces zero tokens, and records a separate receipt for a fragment that
//! does not complete (e.g. it panics). Nested [`Group`] tokens are located
//! separately from the fragment's outer range.
//!
//! [`quote!`]: crate::quote
//! [`quote_observe!`]: crate::quote_observe

use crate::ToTokens;
use alloc::vec::Vec;
use core::ops::Range;
use proc_macro2::{Group, TokenStream, TokenTree};

/// A source identifier, uniquely identifying an interpolation fragment.
///
/// Each call to a [`ToTokens`] implementation inside an observed [`quote!`]
/// invocation is assigned a unique `SourceId`. Identifiers are allocated in
/// observation order, so the first fragment observed has id `0`, the second
/// `1`, and so on.
///
/// [`quote!`]: crate::quote
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SourceId(usize);

impl SourceId {
    /// The raw numeric identifier.
    pub fn index(self) -> usize {
        self.0
    }
}

/// A range of top-level tokens in the output [`TokenStream`].
///
/// The range is measured in top-level token indices (the same order as
/// [`TokenStream::into_iter`]). A [`Group`] counts as a single top-level
/// token; its inner tokens are recorded separately via [`GroupRecord`].
pub type TokenRange = Range<usize>;

/// The outcome of a single interpolation fragment.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FragmentStatus {
    /// The fragment's [`ToTokens`] implementation returned normally.
    Complete,
    /// The fragment's [`ToTokens`] implementation did not return (e.g. it
    /// panicked).
    Incomplete,
}

/// A record of a single interpolation fragment's output.
#[derive(Clone, Debug)]
pub struct FragmentRecord {
    /// The source identifier of the interpolation.
    pub id: SourceId,
    /// The range of top-level tokens produced by the fragment.
    ///
    /// The range is empty if the fragment produced no tokens.
    pub range: TokenRange,
    /// Whether the fragment completed normally.
    pub status: FragmentStatus,
    /// Nested groups within the fragment's output.
    pub groups: Vec<GroupRecord>,
}

/// A record of a [`Group`] within a fragment's output.
#[derive(Clone, Debug)]
pub struct GroupRecord {
    /// The offset of the group token within the fragment's range.
    pub offset: usize,
    /// The number of tokens in the group's inner stream.
    pub inner_len: usize,
    /// Nested groups within the group's inner stream.
    pub inner_groups: Vec<GroupRecord>,
}

/// An observer for [`ToTokens`] interpolations.
///
/// An observer is notified as each interpolation fragment is processed. The
/// observer can record the [`SourceId`], output [`TokenRange`], and
/// completion status of each fragment.
pub trait Observer {
    /// Allocate the next source identifier.
    fn next_id(&mut self) -> SourceId;

    /// Record that a fragment started.
    ///
    /// This is called before the fragment's [`ToTokens`] implementation is
    /// invoked.
    fn on_fragment_start(&mut self, id: SourceId);

    /// Record that a fragment's [`ToTokens`] implementation returned
    /// normally.
    fn on_fragment_complete(
        &mut self,
        id: SourceId,
        range: TokenRange,
        groups: Vec<GroupRecord>,
    );

    /// Record that a fragment's [`ToTokens`] implementation did not return.
    ///
    /// This is called if the fragment's `to_tokens` panics. The fragment is
    /// recorded as incomplete, not as a complete stream.
    fn on_fragment_incomplete(&mut self, id: SourceId);
}

impl<O: Observer + ?Sized> Observer for &mut O {
    fn next_id(&mut self) -> SourceId {
        (**self).next_id()
    }

    fn on_fragment_start(&mut self, id: SourceId) {
        (**self).on_fragment_start(id)
    }

    fn on_fragment_complete(
        &mut self,
        id: SourceId,
        range: TokenRange,
        groups: Vec<GroupRecord>,
    ) {
        (**self).on_fragment_complete(id, range, groups)
    }

    fn on_fragment_incomplete(&mut self, id: SourceId) {
        (**self).on_fragment_incomplete(id)
    }
}

/// A simple observer that records all fragments for later inspection.
///
/// # Example
///
/// ```
/// use quote::observe::{FragmentStatus, Observer, RecordingObserver};
///
/// let mut observer = RecordingObserver::new();
/// let id = observer.next_id();
/// observer.on_fragment_start(id);
/// assert_eq!(observer.fragments[0].status, FragmentStatus::Incomplete);
/// ```
#[derive(Default, Debug)]
pub struct RecordingObserver {
    next_id: usize,
    /// Records in observation order.
    pub fragments: Vec<FragmentRecord>,
}

impl RecordingObserver {
    /// Create a new, empty recording observer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if every observed fragment completed normally.
    pub fn all_complete(&self) -> bool {
        self.fragments
            .iter()
            .all(|f| f.status == FragmentStatus::Complete)
    }

    /// Look up the record for a given source identifier.
    pub fn get(&self, id: SourceId) -> Option<&FragmentRecord> {
        self.fragments.iter().find(|f| f.id == id)
    }
}

impl Observer for RecordingObserver {
    fn next_id(&mut self) -> SourceId {
        let id = SourceId(self.next_id);
        self.next_id += 1;
        id
    }

    fn on_fragment_start(&mut self, id: SourceId) {
        self.fragments.push(FragmentRecord {
            id,
            range: 0..0,
            status: FragmentStatus::Incomplete,
            groups: Vec::new(),
        });
    }

    fn on_fragment_complete(
        &mut self,
        id: SourceId,
        range: TokenRange,
        groups: Vec<GroupRecord>,
    ) {
        if let Some(rec) = self.fragments.iter_mut().find(|r| r.id == id) {
            rec.range = range;
            rec.groups = groups;
            rec.status = FragmentStatus::Complete;
        }
    }

    fn on_fragment_incomplete(&mut self, id: SourceId) {
        if let Some(rec) = self.fragments.iter_mut().find(|r| r.id == id) {
            rec.status = FragmentStatus::Incomplete;
        }
    }
}

/// An observed token stream.
///
/// Wraps a [`TokenStream`] and an [`Observer`]. The observed quote macros
/// append template tokens to the inner stream (via [`DerefMut`]) and call
/// [`interpolate`] for each interpolation fragment.
///
/// [`DerefMut`]: core::ops::DerefMut
pub struct ObservedStream<O> {
    inner: TokenStream,
    observer: O,
}

impl<O: Observer> ObservedStream<O> {
    /// Create a new observed stream with the given observer.
    pub fn new(observer: O) -> Self {
        ObservedStream {
            inner: TokenStream::new(),
            observer,
        }
    }

    /// Consume the stream and return the inner [`TokenStream`].
    pub fn into_inner(self) -> TokenStream {
        self.inner
    }

    /// Consume the stream and return the observer.
    pub fn into_observer(self) -> O {
        self.observer
    }
}

impl<O> core::ops::Deref for ObservedStream<O> {
    type Target = TokenStream;
    fn deref(&self) -> &TokenStream {
        &self.inner
    }
}

impl<O> core::ops::DerefMut for ObservedStream<O> {
    fn deref_mut(&mut self) -> &mut TokenStream {
        &mut self.inner
    }
}

/// Instrument a [`ToTokens`] implementation, recording its output range.
///
/// This function is called by the observed quote macros for each
/// interpolation fragment. It calls [`ToTokens::to_tokens`] exactly once and
/// records the range of top-level tokens produced. The range is measured
/// explicitly (by counting tokens before and after), not guessed from the
/// final string.
pub fn interpolate<O: Observer, T: ToTokens>(stream: &mut ObservedStream<O>, var: &T) {
    let id = stream.observer.next_id();
    stream.observer.on_fragment_start(id);
    let start = count_tokens(&stream.inner);

    let mut guard = IncompleteGuard {
        observer: &mut stream.observer,
        id,
        done: false,
    };

    var.to_tokens(&mut stream.inner);

    let end = count_tokens(&stream.inner);
    let groups = find_groups(&stream.inner, start, end);

    guard.observer.on_fragment_complete(id, start..end, groups);
    guard.done = true;
}

/// A guard that marks a fragment incomplete if `to_tokens` panics.
struct IncompleteGuard<'a, O: Observer> {
    observer: &'a mut O,
    id: SourceId,
    done: bool,
}

impl<'a, O: Observer> Drop for IncompleteGuard<'a, O> {
    fn drop(&mut self) {
        if !self.done {
            self.observer.on_fragment_incomplete(self.id);
        }
    }
}

/// Count the number of top-level tokens in a [`TokenStream`].
fn count_tokens(stream: &TokenStream) -> usize {
    stream.clone().into_iter().count()
}

/// Find groups within a fragment's output range.
fn find_groups(stream: &TokenStream, start: usize, end: usize) -> Vec<GroupRecord> {
    let mut groups = Vec::new();
    let len = end.saturating_sub(start);
    for (i, tt) in stream.clone().into_iter().enumerate().skip(start).take(len) {
        if let TokenTree::Group(g) = tt {
            groups.push(walk_group(&g, i - start));
        }
    }
    groups
}

/// Walk a group, recording its inner structure.
fn walk_group(g: &Group, offset: usize) -> GroupRecord {
    let inner: Vec<TokenTree> = g.stream().into_iter().collect();
    let inner_groups = inner
        .iter()
        .enumerate()
        .filter_map(|(i, tt)| match tt {
            TokenTree::Group(inner) => Some(walk_group(inner, i)),
            _ => None,
        })
        .collect();
    GroupRecord {
        offset,
        inner_len: inner.len(),
        inner_groups,
    }
}

/// Same as [`quote!`], but observes each interpolation fragment.
///
/// The macro takes an observer expression (evaluating to `&mut impl
/// Observer`) followed by `=>` and the tokens to quote. Each interpolation
/// (`#var`) is instrumented: its [`ToTokens`] implementation is called exactly
/// once, and the fragment's source id and output token range are recorded in
/// the observer.
///
/// See the [`observe` module documentation] for more detail.
///
/// [`quote!`]: crate::quote
/// [`observe` module documentation]: crate::observe
///
/// # Example
///
/// ```
/// use quote::observe::RecordingObserver;
/// use quote::{quote_observe, ToTokens};
/// use proc_macro2::{Ident, Span, TokenStream};
///
/// let mut observer = RecordingObserver::new();
/// let a = Ident::new("alpha", Span::call_site());
/// let b: Option<TokenStream> = None;
/// let c = Ident::new("gamma", Span::call_site());
/// let tokens = quote_observe! { &mut observer=> #a #b #c };
///
/// assert_eq!(tokens.to_string(), "alpha gamma");
/// assert_eq!(observer.fragments.len(), 3);
/// assert!(observer.all_complete());
/// ```
#[macro_export]
macro_rules! quote_observe {
    ($observer:expr=>) => {{
        let _s = $crate::__private::observe::ObservedStream::new($observer);
        $crate::__private::observe::ObservedStream::into_inner(_s)
    }};

    ($observer:expr=> $($tt:tt)*) => {{
        let mut _s = $crate::__private::observe::ObservedStream::new($observer);
        $crate::quote_each_token_observe!{_s $($tt)*}
        $crate::__private::observe::ObservedStream::into_inner(_s)
    }};
}

// See `quote_each_token!` for an explanation of the transposition.
#[macro_export]
#[doc(hidden)]
macro_rules! quote_each_token_observe {
    ($tokens:ident $($tts:tt)*) => {
        $crate::quote_tokens_with_context_observe!{$tokens
            (@ @ @ @ @ @ $($tts)*)
            (@ @ @ @ @ $($tts)* @)
            (@ @ @ @ $($tts)* @ @)
            (@ @ @ $(($tts))* @ @ @)
            (@ @ $($tts)* @ @ @ @)
            (@ $($tts)* @ @ @ @ @)
            ($($tts)* @ @ @ @ @ @)
        }
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! quote_tokens_with_context_observe {
    ($tokens:ident
        ($($b3:tt)*) ($($b2:tt)*) ($($b1:tt)*)
        ($($curr:tt)*)
        ($($a1:tt)*) ($($a2:tt)*) ($($a3:tt)*)
    ) => {
        $(
            $crate::quote_token_with_context_observe!{$tokens $b3 $b2 $b1 $curr $a1 $a2 $a3}
        )*
    };
}

// See `quote_token_with_context!` for an explanation of each rule. The only
// difference here is that interpolations go through `observe::interpolate`
// instead of calling `ToTokens::to_tokens` directly.
#[macro_export]
#[doc(hidden)]
macro_rules! quote_token_with_context_observe {
    // Unparenthesized `@` indicates this call does not correspond to one of
    // the original input tokens. Ignore it.
    ($tokens:ident $b3:tt $b2:tt $b1:tt @ $a1:tt $a2:tt $a3:tt) => {};

    // A repetition with no separator.
    ($tokens:ident $b3:tt $b2:tt $b1:tt (#) ( $($inner:tt)* ) * $a3:tt) => {{
        use $crate::__private::ext::*;
        let has_iter = $crate::__private::HasIterator::<false>;
        $crate::pounded_var_names!{quote_bind_into_iter!(has_iter) () $($inner)*}
        <_ as $crate::__private::CheckHasIterator<true>>::check(has_iter);
        while true {
            $crate::pounded_var_names!{quote_bind_next_or_break!() () $($inner)*}
            $crate::quote_each_token_observe!{$tokens $($inner)*}
        }
    }};
    ($tokens:ident $b3:tt $b2:tt # (( $($inner:tt)* )) * $a2:tt $a3:tt) => {};
    ($tokens:ident $b3:tt # ( $($inner:tt)* ) (*) $a1:tt $a2:tt $a3:tt) => {};

    // A repetition with separator.
    ($tokens:ident $b3:tt $b2:tt $b1:tt (#) ( $($inner:tt)* ) $sep:tt *) => {{
        use $crate::__private::ext::*;
        let mut _first = true;
        let has_iter = $crate::__private::HasIterator::<false>;
        $crate::pounded_var_names!{quote_bind_into_iter!(has_iter) () $($inner)*}
        <_ as $crate::__private::CheckHasIterator<true>>::check(has_iter);
        while true {
            $crate::pounded_var_names!{quote_bind_next_or_break!() () $($inner)*}
            if !_first {
                $crate::quote_token!{$sep $tokens}
            }
            _first = false;
            $crate::quote_each_token_observe!{$tokens $($inner)*}
        }
    }};
    ($tokens:ident $b3:tt $b2:tt # (( $($inner:tt)* )) $sep:tt * $a3:tt) => {};
    ($tokens:ident $b3:tt # ( $($inner:tt)* ) ($sep:tt) * $a2:tt $a3:tt) => {};
    ($tokens:ident # ( $($inner:tt)* ) * (*) $a1:tt $a2:tt $a3:tt) => {
        // https://github.com/dtolnay/quote/issues/130
        $crate::quote_token!{* $tokens}
    };
    ($tokens:ident # ( $($inner:tt)* ) $sep:tt (*) $a1:tt $a2:tt $a3:tt) => {};

    // A non-repetition interpolation. This is the only rule that differs from
    // the non-observed path: we go through `observe::interpolate` so that the
    // fragment's source id and output range are recorded.
    ($tokens:ident $b3:tt $b2:tt $b1:tt (#) $var:ident $a2:tt $a3:tt) => {
        $crate::__private::observe::interpolate(&mut $tokens, &$var);
    };
    ($tokens:ident $b3:tt $b2:tt # ($var:ident) $a1:tt $a2:tt $a3:tt) => {};

    // An ordinary token, not part of any interpolation. These are appended to
    // the inner token stream directly (via `DerefMut`), preserving their
    // original spacing.
    ($tokens:ident $b3:tt $b2:tt $b1:tt ($curr:tt) $a1:tt $a2:tt $a3:tt) => {
        $crate::quote_token!{$curr $tokens}
    };
}
