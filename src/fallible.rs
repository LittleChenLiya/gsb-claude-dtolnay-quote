use crate::ToTokens;
use core::fmt;
use proc_macro2::TokenStream;

/// Failure returned by [`try_join_tokens`].
///
/// Carries the index of the element that failed (counted from 0 over the
/// original input collection, including empty fragments) and the original
/// cause. No partially built `TokenStream` is exposed: the fragments and
/// separators that were already assembled are discarded together with the
/// internal buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinFailure<E> {
    /// Index of the failed element in the input collection.
    pub index: usize,
    /// The error produced by the failed element.
    pub cause: E,
}

impl<E> JoinFailure<E> {
    /// Map the cause to a different error type, keeping the index.
    pub fn map_cause<F>(self, f: impl FnOnce(E) -> F) -> JoinFailure<F> {
        JoinFailure {
            index: self.index,
            cause: f(self.cause),
        }
    }
}

impl<E: fmt::Display> fmt::Display for JoinFailure<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(
            formatter,
            "fragment at index {} failed to produce tokens: {}",
            self.index, self.cause,
        )
    }
}

impl<E> std::error::Error for JoinFailure<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// Join a collection of fallible [`ToTokens`] fragments into a single
/// `TokenStream`, emitting `separator` only between two successful non-empty
/// fragments.
///
/// This is the short-circuiting analogue of writing `#(#fragments),*` inside
/// `quote!`: interpolation through `quote!` is unchanged and remains the
/// right tool for infallible fragments, while this helper covers collections
/// whose elements are produced as `Result<T, E>` with `T: ToTokens`.
///
/// Semantics:
///
/// - Elements are consumed in order. The first `Err` stops the join
///   immediately; later elements are never pulled from the iterator and
///   their `to_tokens` is never called.
/// - A separator is emitted only when a non-empty fragment has already been
///   written and the next successful fragment is also non-empty. Fragments
///   that tokenize to nothing never attract a separator on either side.
/// - On failure the function returns [`JoinFailure`] with the element index
///   and the original cause. The partially assembled tokens are dropped, so
///   no half-built `TokenStream` can leak to the caller.
///
/// # Example
///
/// ```
/// use proc_macro2::{Ident, Span};
/// use quote::try_join_tokens;
///
/// let fields: Vec<Result<Ident, String>> = vec![
///     Ok(Ident::new("a", Span::call_site())),
///     Ok(Ident::new("b", Span::call_site())),
/// ];
/// let joined = try_join_tokens(fields, quote::quote!(,)).unwrap();
/// assert_eq!(joined.to_string(), "a , b");
///
/// let fields: Vec<Result<Ident, String>> = vec![
///     Ok(Ident::new("a", Span::call_site())),
///     Err("bad field".to_owned()),
/// ];
/// let failure = try_join_tokens(fields, quote::quote!(,)).unwrap_err();
/// assert_eq!(failure.index, 1);
/// assert_eq!(failure.cause, "bad field");
/// ```
pub fn try_join_tokens<T, E, S, I>(fragments: I, separator: S) -> Result<TokenStream, JoinFailure<E>>
where
    T: ToTokens,
    S: ToTokens,
    I: IntoIterator<Item = Result<T, E>>,
{
    let mut out = TokenStream::new();
    let mut needs_separator = false;
    for (index, fragment) in fragments.into_iter().enumerate() {
        let fragment = match fragment {
            Ok(fragment) => fragment,
            Err(cause) => return Err(JoinFailure { index, cause }),
        };
        let mut buf = TokenStream::new();
        fragment.to_tokens(&mut buf);
        if buf.is_empty() {
            continue;
        }
        if needs_separator {
            separator.to_tokens(&mut out);
        }
        out.extend(buf);
        needs_separator = true;
    }
    Ok(out)
}
