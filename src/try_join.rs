//! Fallible collection joining.
//!
//! This module is enabled by the `fallible` feature and provides a
//! [`TryToTokens`] trait for types whose interpolation can fail, together with
//! short-circuiting [`try_join`] / [`try_join_separated`] helpers and the
//! [`TryTokenStreamExt`] extension trait.
//!
//! Unlike [`ToTokens`] (which cannot report failure), a [`TryToTokens`]
//! implementation returns a `Result`. The joining helpers iterate over a
//! collection of such fragments, and:
//!
//! - a fragment that produces no tokens contributes nothing;
//! - the separator is emitted only *between* two fragments that both succeed
//!   and produce non-empty token streams, never next to an empty fragment and
//!   not before the first or after the last non-empty one;
//! - on the first failing fragment iteration stops immediately: later
//!   fragments are never visited and the returned error carries the zero-based
//!   index of the failing element together with its cause;
//! - on failure no partial token stream is ever produced. Fragment tokens are
//!   accumulated in a private staging stream that is discarded on error; for
//!   the `try_append_*` methods the caller's destination stream is left
//!   untouched.
//!
//! Every type that implements [`ToTokens`] also implements [`TryToTokens`]
//! with [`Infallible`] as the error type, so infallible values (integers,
//! identifiers, quoted streams, and so on) can be joined directly.

use crate::ToTokens;
use core::convert::Infallible;
use core::fmt;
use core::mem;
use proc_macro2::TokenStream;

/// Types that can be interpolated, but may fail while producing their tokens.
///
/// This is the fallible counterpart of [`ToTokens`]. A blanket implementation
/// is provided for every `T: ToTokens`, with [`Infallible`] as the `Error`
/// type, so existing `ToTokens` implementations (and therefore the ordinary
/// `quote!` interpolation behavior) are usable wherever `TryToTokens` is
/// expected.
///
/// Implement this trait for a custom type when generating its tokens depends
/// on something that can fail (a lookup, a validation, an identifier parse,
/// and so on) and the failure must short-circuit a `try_join`/`try_append_*`
/// operation instead of panicking.
///
/// # Example
///
/// ```
/// # #[cfg(feature = "fallible")]
/// # fn main() {
/// use proc_macro2::{Ident, Span, TokenStream};
/// use quote::{ToTokens, TryToTokens};
///
/// struct MaybeIdent(Option<&'static str>);
///
/// impl TryToTokens for MaybeIdent {
///     type Error = &'static str;
///
///     fn try_to_tokens(&self, tokens: &mut TokenStream) -> Result<(), Self::Error> {
///         let name = self.0.ok_or("missing identifier")?;
///         Ident::new(name, Span::call_site()).to_tokens(tokens);
///         Ok(())
///     }
/// }
/// # let _ = MaybeIdent(Some("x")).try_to_token_stream();
/// # }
/// # #[cfg(not(feature = "fallible"))]
/// # fn main() {}
/// ```
pub trait TryToTokens {
    /// The error returned when tokenization fails.
    type Error;

    /// Try to write `self` to the given `TokenStream`.
    ///
    /// On success the generated tokens (possibly none) have been appended to
    /// `tokens`. On error `tokens` may contain a partial prefix produced before
    /// the failure; the joining helpers in this module take care never to
    /// expose such a partial stream to their callers.
    fn try_to_tokens(&self, tokens: &mut TokenStream) -> Result<(), Self::Error>;

    /// Try to convert `self` directly into a `TokenStream`.
    fn try_to_token_stream(&self) -> Result<TokenStream, Self::Error> {
        let mut tokens = TokenStream::new();
        self.try_to_tokens(&mut tokens)?;
        Ok(tokens)
    }
}

impl<T: ToTokens + ?Sized> TryToTokens for T {
    type Error = Infallible;

    fn try_to_tokens(&self, tokens: &mut TokenStream) -> Result<(), Infallible> {
        self.to_tokens(tokens);
        Ok(())
    }
}

/// Error returned by the fallible joining helpers when a fragment fails.
///
/// Contains the zero-based `index` of the failing element within the input
/// collection and the `cause` produced by that element
/// [`TryToTokens::try_to_tokens`] implementation.
pub struct JoinError<E> {
    /// Zero-based index of the failing element.
    index: usize,
    /// Error produced by the failing element.
    cause: E,
}

impl<E> JoinError<E> {
    /// Returns the zero-based index of the element that failed.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Returns a reference to the cause produced by the failing element.
    pub fn cause(&self) -> &E {
        &self.cause
    }

    /// Returns the cause produced by the failing element, consuming the error.
    pub fn into_cause(self) -> E {
        self.cause
    }
}

impl<E> From<JoinError<E>> for (usize, E) {
    fn from(error: JoinError<E>) -> Self {
        (error.index, error.cause)
    }
}

impl<E: fmt::Display> fmt::Display for JoinError<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "failed to join token fragment at index {}: {}",
            self.index, self.cause,
        )
    }
}

impl<E: fmt::Debug> fmt::Debug for JoinError<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("JoinError")
            .field("index", &self.index)
            .field("cause", &self.cause)
            .finish()
    }
}

impl<E: std::error::Error + 'static> std::error::Error for JoinError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// Joins every fragment into one [`TokenStream`], with no separator.
///
/// Each fragment must implement [`TryToTokens`]. Empty fragments contribute
/// nothing. On the first failing fragment, iteration stops immediately and the
/// returned [`JoinError`] identifies the failing element by index; later
/// elements are not visited and no partial token stream is returned.
pub fn try_join<I>(fragments: I) -> Result<TokenStream, JoinError<<I::Item as TryToTokens>::Error>>
where
    I: IntoIterator,
    I::Item: TryToTokens,
{
    let mut out = TokenStream::new();
    join_into(&mut out, fragments, None)?;
    Ok(out)
}

/// Joins every fragment into one [`TokenStream`], placing `separator` between
/// two successful non-empty fragments.
///
/// The separator is emitted at most once per pair of adjacent non-empty
/// fragments: empty fragments are skipped silently and never attract a
/// separator. On the first failing fragment, iteration stops immediately and
/// the returned [`JoinError`] identifies the failing element by index; later
/// elements are not visited and no partial token stream is returned.
pub fn try_join_separated<I, S>(
    fragments: I,
    separator: S,
) -> Result<TokenStream, JoinError<<I::Item as TryToTokens>::Error>>
where
    I: IntoIterator,
    I::Item: TryToTokens,
    S: ToTokens,
{
    let mut out = TokenStream::new();
    let separator = separator.to_token_stream();
    join_into(&mut out, fragments, Some(&separator))?;
    Ok(out)
}

/// Appends a fallible collection of fragments onto an existing
/// [`TokenStream`], transactionally.
///
/// See [`try_join_separated`] for the separator, empty-fragment and
/// short-circuit semantics. If any fragment fails, the destination stream is
/// left exactly as it was before the call and a [`JoinError`] is returned; no
/// partial tokens are ever appended.
///
/// This trait is sealed and cannot be implemented outside of the `quote`
/// crate.
pub trait TryTokenStreamExt: private::Sealed {
    /// Tries to append every fragment onto this stream, with no separator.
    ///
    /// On failure this stream is left untouched.
    fn try_append_all<I>(&mut self, fragments: I) -> Result<(), JoinError<<I::Item as TryToTokens>::Error>>
    where
        I: IntoIterator,
        I::Item: TryToTokens;

    /// Tries to append every fragment onto this stream, placing `separator`
    /// between two successful non-empty fragments.
    ///
    /// Empty fragments do not attract separators. On failure this stream is
    /// left untouched and no partial tokens are appended.
    fn try_append_separated<I, S>(
        &mut self,
        fragments: I,
        separator: S,
    ) -> Result<(), JoinError<<I::Item as TryToTokens>::Error>>
    where
        I: IntoIterator,
        I::Item: TryToTokens,
        S: ToTokens;
}

impl TryTokenStreamExt for TokenStream {
    fn try_append_all<I>(&mut self, fragments: I) -> Result<(), JoinError<<I::Item as TryToTokens>::Error>>
    where
        I: IntoIterator,
        I::Item: TryToTokens,
    {
        join_into(self, fragments, None)
    }

    fn try_append_separated<I, S>(
        &mut self,
        fragments: I,
        separator: S,
    ) -> Result<(), JoinError<<I::Item as TryToTokens>::Error>>
    where
        I: IntoIterator,
        I::Item: TryToTokens,
        S: ToTokens,
    {
        let separator = separator.to_token_stream();
        join_into(self, fragments, Some(&separator))
    }
}

/// Core short-circuiting loop shared by every entry point.
///
/// All tokens are built in `staging`; it is merged into `out` only after every
/// fragment has succeeded, so failure leaves `out` untouched.
fn join_into<I>(
    out: &mut TokenStream,
    fragments: I,
    separator: Option<&TokenStream>,
) -> Result<(), JoinError<<I::Item as TryToTokens>::Error>>
where
    I: IntoIterator,
    I::Item: TryToTokens,
{
    let mut staging = TokenStream::new();
    let mut emitted = 0usize;

    for (index, fragment) in fragments.into_iter().enumerate() {
        let mut scratch = TokenStream::new();
        match fragment.try_to_tokens(&mut scratch) {
            Ok(()) => {
                if scratch.is_empty() {
                    continue;
                }
                if emitted > 0 {
                    if let Some(separator) = separator {
                        staging.extend(separator.clone());
                    }
                }
                staging.extend(mem::take(&mut scratch));
                emitted += 1;
            }
            Err(cause) => {
                return Err(JoinError { index, cause });
            }
        }
    }

    // Every fragment succeeded: commit the staged output atomically.
    out.extend(staging);
    Ok(())
}

mod private {
    use proc_macro2::TokenStream;

    pub trait Sealed {}

    impl Sealed for TokenStream {}
}
