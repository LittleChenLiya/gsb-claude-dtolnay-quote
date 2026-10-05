//! Explicit observation of `ToTokens` interpolation fragments.
//!
//! When `quote!` interpolates `#var`, the contribution of each fragment to
//! the final token stream is normally invisible: once the tokens are merged
//! there is no way to tell which bytes came from which interpolation without
//! guessing from the final string. This module provides an explicit observer
//! that binds every interpolation fragment to a caller-chosen [`FragmentId`]
//! and records the exact output token range that the fragment produced.
//!
//! Guarantees:
//!
//! - The wrapped [`ToTokens`] implementation is invoked **exactly once**.
//! - A fragment that emits **zero tokens** still leaves a source receipt: a
//!   [`Fragment`] record with an empty `start..start` range.
//! - Groups recorded through [`trace_group`] store the group's own token
//!   range in the enclosing stream **separately** from the token range of
//!   its inner contents, so nested groups can be located precisely.
//! - A failed or unfinished observation is never reported as a complete
//!   stream: [`Observation::into_stream`] returns [`IncompleteStream`]
//!   instead of the tokens.
//! - The default [`ToTokens`] behavior is unchanged; observation is a
//!   strictly additive wrapper.
//!
//! A fragment's range is recorded in the stream context that is open while
//! the fragment's `ToTokens` implementation runs. Groups written literally
//! inside `quote!` are anonymous and do not open a new context; use
//! [`trace_group`] to give a group its own id and inner range.
//!
//! # Three-interpolation example
//!
//! ```
//! use proc_macro2::{Delimiter, Ident, Span, TokenStream};
//! use quote::observe::{self, FragmentId, Observer};
//! use quote::{quote, ToTokens};
//! use std::cell::RefCell;
//!
//! const NAME: FragmentId = FragmentId::new(0);
//! const GENERICS: FragmentId = FragmentId::new(1);
//! const BODY: FragmentId = FragmentId::new(2);
//!
//! // A fragment that emits zero tokens.
//! struct NoGenerics;
//! impl ToTokens for NoGenerics {
//!     fn to_tokens(&self, _tokens: &mut TokenStream) {}
//! }
//!
//! let observer = RefCell::new(Observer::new());
//! let name = Ident::new("Point", Span::call_site());
//! let fields = quote!(x: u8, y: u8);
//!
//! let name_t = observe::trace(NAME, &name, &observer);
//! let generics_t = observe::trace(GENERICS, &NoGenerics, &observer);
//! let body_t = observe::trace_group(BODY, Delimiter::Brace, &fields, &observer);
//!
//! let tokens = quote!(struct #name_t #generics_t #body_t);
//! assert_eq!(tokens.to_string(), "struct Point { x : u8 , y : u8 }");
//!
//! let observer = observer.borrow();
//! assert!(observer.is_complete());
//! assert_eq!(observer.fragment(NAME).unwrap().range, 1..2);
//! // Zero output still leaves a source receipt.
//! assert_eq!(observer.fragment(GENERICS).unwrap().range, 2..2);
//! // The brace group is one token in the root stream...
//! assert_eq!(observer.fragment(BODY).unwrap().range, 2..3);
//! // ...and its inner range is recorded separately.
//! assert_eq!(observer.fragment(BODY).unwrap().inner.as_ref().unwrap().range, 0..7);
//! ```
//!
//! # Focused local verification
//!
//! ```console
//! cargo test --test observe
//! ```

use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt;
use core::ops::Range;

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};

use crate::ToTokens;

/// Identifier bound by the caller to one interpolation fragment.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FragmentId(pub u32);

impl FragmentId {
    /// Create a fragment id from a `u32`.
    pub const fn new(id: u32) -> Self {
        FragmentId(id)
    }
}

impl From<u32> for FragmentId {
    fn from(id: u32) -> Self {
        FragmentId(id)
    }
}

impl fmt::Display for FragmentId {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// What kind of source produced a recorded fragment.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FragmentKind {
    /// A `#var` interpolation recorded through [`trace`].
    Interpolation,
    /// A delimited group recorded through [`trace_group`]. The fragment's
    /// `range` covers the single group token in the enclosing stream, while
    /// the inner tokens are recorded separately in [`Fragment::inner`].
    Group(Delimiter),
}

/// Token range of the contents inside a recorded group.
///
/// The range applies to the group's own inner token stream, which is tracked
/// as a separate stream context ([`GroupInner::stream`]) so nested groups can
/// be located without inspecting the final string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupInner {
    /// Stream context of the group's inner tokens.
    pub stream: usize,
    /// Token range of the contents within that inner stream.
    pub range: Range<usize>,
}

/// One recorded source receipt: which fragment produced which output tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fragment {
    /// Caller-bound source id of the fragment.
    pub id: FragmentId,
    /// Whether this was an interpolation or a delimited group.
    pub kind: FragmentKind,
    /// Stream context the `range` applies to. The root output stream is `0`;
    /// each recorded group opens a fresh context for its inner tokens.
    pub stream: usize,
    /// Token range contributed by this fragment within `stream`. Empty when
    /// the fragment emitted zero tokens.
    pub range: Range<usize>,
    /// For groups: the token range of the contents inside the group, recorded
    /// separately from the group's own `range`.
    pub inner: Option<GroupInner>,
}

#[derive(Debug)]
struct OpenFragment {
    id: FragmentId,
    kind: FragmentKind,
    stream: usize,
    start: usize,
    child_stream: Option<usize>,
}

/// Records source receipts for observed interpolation fragments.
///
/// Use through the [`trace`] and [`trace_group`] wrappers, or drive a whole
/// quotation with [`observe`]. The observer never alters the tokens being
/// written; it only records ranges alongside them.
#[derive(Debug, Default)]
pub struct Observer {
    fragments: Vec<Fragment>,
    open: Vec<OpenFragment>,
    stream_stack: Vec<usize>,
    next_stream: usize,
    poisoned: bool,
}

impl Observer {
    /// Create an empty observer rooted at stream context `0`.
    pub fn new() -> Self {
        Observer {
            fragments: Vec::new(),
            open: Vec::new(),
            stream_stack: vec![0],
            next_stream: 1,
            poisoned: false,
        }
    }

    /// All fragments recorded so far, in completion order.
    pub fn fragments(&self) -> &[Fragment] {
        &self.fragments
    }

    /// Look up the receipt of one fragment by its source id.
    pub fn fragment(&self, id: FragmentId) -> Option<&Fragment> {
        self.fragments.iter().find(|fragment| fragment.id == id)
    }

    /// Whether every begun fragment finished and none failed.
    ///
    /// Only a complete observer entitles the produced tokens to be treated as
    /// a complete stream; see [`Observation::into_stream`].
    pub fn is_complete(&self) -> bool {
        !self.poisoned && self.open.is_empty() && self.stream_stack.len() == 1
    }

    /// Record the start of an interpolation fragment at token position
    /// `start` of the currently open stream context.
    ///
    /// Normally called by [`Traced`]; public so custom wrappers can integrate
    /// with the same receipts.
    pub fn begin(&mut self, id: FragmentId, start: usize) {
        let stream = self.current_stream();
        self.open.push(OpenFragment {
            id,
            kind: FragmentKind::Interpolation,
            stream,
            start,
            child_stream: None,
        });
    }

    /// Record the end of the most recently begun interpolation fragment at
    /// token position `end`. Emits the [`Fragment`] receipt, even when
    /// `end == start` (zero output).
    pub fn end(&mut self, end: usize) {
        match self.open.pop() {
            Some(open) if open.child_stream.is_none() => {
                self.fragments.push(Fragment {
                    id: open.id,
                    kind: open.kind,
                    stream: open.stream,
                    range: open.start..end,
                    inner: None,
                });
            }
            _ => self.poisoned = true,
        }
    }

    /// Record the start of a delimited group at token position `start` of the
    /// currently open stream context, and open a fresh stream context for the
    /// group's inner tokens.
    pub fn begin_group(&mut self, id: FragmentId, delimiter: Delimiter, start: usize) {
        let stream = self.current_stream();
        let child = self.next_stream;
        self.next_stream += 1;
        self.stream_stack.push(child);
        self.open.push(OpenFragment {
            id,
            kind: FragmentKind::Group(delimiter),
            stream,
            start,
            child_stream: Some(child),
        });
    }

    /// Record the end of the most recently begun group. `inner_len` is the
    /// number of tokens inside the group and `end` is the token position just
    /// past the group token in the enclosing stream. The group's own range
    /// and its inner range are recorded separately.
    pub fn end_group(&mut self, inner_len: usize, end: usize) {
        match self.open.pop() {
            Some(open) => {
                if let Some(child) = open.child_stream {
                    self.stream_stack.pop();
                    self.fragments.push(Fragment {
                        id: open.id,
                        kind: open.kind,
                        stream: open.stream,
                        range: open.start..end,
                        inner: Some(GroupInner {
                            stream: child,
                            range: 0..inner_len,
                        }),
                    });
                } else {
                    self.poisoned = true;
                }
            }
            None => self.poisoned = true,
        }
    }

    /// Mark the observation as failed. A poisoned observer never reports
    /// [`is_complete`](Observer::is_complete), so its tokens are never
    /// mistaken for a complete stream.
    pub fn fail(&mut self) {
        self.poisoned = true;
    }

    fn current_stream(&self) -> usize {
        *self.stream_stack.last().unwrap_or(&0)
    }
}

/// Marks the observation as failed if a fragment is dropped before its
/// receipt was recorded, e.g. when a `ToTokens` implementation panics.
struct FragmentGuard<'a> {
    observer: &'a RefCell<Observer>,
    completed: bool,
}

impl FragmentGuard<'_> {
    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for FragmentGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.observer.borrow_mut().fail();
        }
    }
}

fn stream_len(stream: &TokenStream) -> usize {
    stream.clone().into_iter().count()
}

/// A `ToTokens` wrapper that binds one interpolation fragment to a
/// [`FragmentId`] and records its output token range in an [`Observer`].
///
/// The wrapped value's `ToTokens` implementation is invoked exactly once per
/// interpolation, and even a zero-token output produces a receipt.
pub struct Traced<'a, T: ?Sized> {
    id: FragmentId,
    value: &'a T,
    observer: &'a RefCell<Observer>,
}

/// Bind `value` to the source `id` so interpolating the returned wrapper
/// records a receipt in `observer`.
pub fn trace<'a, T: ?Sized>(
    id: FragmentId,
    value: &'a T,
    observer: &'a RefCell<Observer>,
) -> Traced<'a, T> {
    Traced {
        id,
        value,
        observer,
    }
}

impl<T: ToTokens + ?Sized> ToTokens for Traced<'_, T> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let start = stream_len(tokens);
        self.observer.borrow_mut().begin(self.id, start);
        let mut guard = FragmentGuard {
            observer: self.observer,
            completed: false,
        };
        self.value.to_tokens(tokens);
        let end = stream_len(tokens);
        self.observer.borrow_mut().end(end);
        guard.complete();
    }
}

/// A `ToTokens` wrapper that emits its value as a delimited group and records
/// the group's token range separately from the range of its inner tokens.
pub struct TracedGroup<'a, T: ?Sized> {
    id: FragmentId,
    delimiter: Delimiter,
    value: &'a T,
    observer: &'a RefCell<Observer>,
}

/// Bind `value` to the source `id` so interpolating the returned wrapper
/// emits a `delimiter` group whose outer and inner token ranges are recorded
/// separately in `observer`.
pub fn trace_group<'a, T: ?Sized>(
    id: FragmentId,
    delimiter: Delimiter,
    value: &'a T,
    observer: &'a RefCell<Observer>,
) -> TracedGroup<'a, T> {
    TracedGroup {
        id,
        delimiter,
        value,
        observer,
    }
}

impl<T: ToTokens + ?Sized> ToTokens for TracedGroup<'_, T> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let start = stream_len(tokens);
        self.observer
            .borrow_mut()
            .begin_group(self.id, self.delimiter, start);
        let mut guard = FragmentGuard {
            observer: self.observer,
            completed: false,
        };
        let mut inner = TokenStream::new();
        self.value.to_tokens(&mut inner);
        let inner_len = stream_len(&inner);
        tokens.extend([TokenTree::Group(Group::new(self.delimiter, inner))]);
        let end = stream_len(tokens);
        self.observer.borrow_mut().end_group(inner_len, end);
        guard.complete();
    }
}

/// The result of [`observe`]: the produced tokens plus every recorded
/// fragment receipt.
pub struct Observation {
    stream: TokenStream,
    observer: Observer,
}

impl Observation {
    /// The produced tokens. These are only a *complete* stream when
    /// [`is_complete`](Observation::is_complete) holds.
    pub fn stream(&self) -> &TokenStream {
        &self.stream
    }

    /// The observer holding all fragment receipts.
    pub fn observer(&self) -> &Observer {
        &self.observer
    }

    /// All recorded fragment receipts.
    pub fn fragments(&self) -> &[Fragment] {
        self.observer.fragments()
    }

    /// Whether every observed fragment finished successfully.
    pub fn is_complete(&self) -> bool {
        self.observer.is_complete()
    }

    /// Consume the observation and return the tokens as a complete stream.
    ///
    /// Returns [`IncompleteStream`] — never the tokens — if any fragment
    /// failed or was left unfinished.
    pub fn into_stream(self) -> Result<TokenStream, IncompleteStream> {
        if self.observer.is_complete() {
            Ok(self.stream)
        } else {
            Err(IncompleteStream {
                stream: self.stream,
                observer: self.observer,
            })
        }
    }

    /// Split into tokens and observer regardless of completeness.
    pub fn into_parts(self) -> (TokenStream, Observer) {
        (self.stream, self.observer)
    }
}

/// A failed or unfinished observation. Deliberately *not* a stream: the
/// partial tokens and the recorded receipts stay accessible through
/// [`into_parts`](IncompleteStream::into_parts) for diagnosis.
#[derive(Debug)]
pub struct IncompleteStream {
    stream: TokenStream,
    observer: Observer,
}

impl IncompleteStream {
    /// Recover the partial tokens and the observer with the receipts that
    /// were recorded before the failure.
    pub fn into_parts(self) -> (TokenStream, Observer) {
        (self.stream, self.observer)
    }
}

impl fmt::Display for IncompleteStream {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter
            .write_str("observed token stream is incomplete: a fragment failed or did not finish")
    }
}

impl std::error::Error for IncompleteStream {}

/// Run `build` with a fresh [`Observer`] and collect the produced tokens
/// together with every fragment receipt.
///
/// ```
/// # use proc_macro2::{Ident, Span};
/// # use quote::observe::{self, FragmentId};
/// # use quote::quote;
/// #
/// let observation = observe::observe(|observer| {
///     let name = Ident::new("answer", Span::call_site());
///     let name = observe::trace(FragmentId::new(0), &name, observer);
///     quote!(let #name = 42;)
/// });
/// assert!(observation.is_complete());
/// let stream = observation.into_stream().unwrap();
/// assert_eq!(stream.to_string(), "let answer = 42 ;");
/// ```
pub fn observe(build: impl FnOnce(&RefCell<Observer>) -> TokenStream) -> Observation {
    let observer = RefCell::new(Observer::new());
    let stream = build(&observer);
    Observation {
        stream,
        observer: observer.into_inner(),
    }
}
