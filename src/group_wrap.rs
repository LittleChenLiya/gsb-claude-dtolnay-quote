use crate::{ToTokens, TokenStreamExt};
use proc_macro2::{Delimiter, Group, Span, TokenStream};

/// Policy describing how the spans of the content tokens of a [`GroupWrap`]
/// are treated when the wrapped fragment is emitted.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ContentSpanPolicy {
    /// Keep the original span of every content token.
    ///
    /// Tokens interpolated from variables keep the spans produced by their
    /// own [`ToTokens`] implementation, and groups already present in the
    /// content keep both their delimiter and their spans. Only the delimiter
    /// of the new outer group receives the synthetic span.
    Preserve,
}

/// Nested group wrapper for a quoted fragment, with a split span policy.
///
/// `GroupWrap` wraps any fragment implementing [`ToTokens`] in one additional
/// [`Group`] when the wrapper is interpolated, and keeps the two span sources
/// of the result separate:
///
/// - the delimiter of the new outer group receives a synthetic span chosen by
///   the caller;
/// - the content tokens are emitted with their original spans, according to
///   the given [`ContentSpanPolicy`].
///
/// Because the content is emitted as-is, groups that already exist inside the
/// fragment keep their delimiter, groups with [`Delimiter::None`] keep their
/// transparent boundary, and variables interpolated into the fragment keep
/// the spans produced by their own [`ToTokens`] implementation; the wrapper
/// never respans content. The wrapper itself implements [`ToTokens`], so it
/// flows through the ordinary `quote!` / `quote_spanned!` interpolation data
/// flow like any other interpolated value.
///
/// # Example
///
/// ```
/// use proc_macro2::{Delimiter, Span};
/// use quote::{quote, ContentSpanPolicy, GroupWrap};
///
/// let variant = quote!(Source);
/// let fragment = quote!(#variant = 0);
///
/// let wrapped = GroupWrap::new(
///     Delimiter::Brace,
///     Span::call_site(),
///     ContentSpanPolicy::Preserve,
///     fragment,
/// );
///
/// // Interpolates as one brace-delimited group.
/// let tokens = quote!(#wrapped);
/// assert_eq!(tokens.to_string(), "{ Source = 0 }");
/// ```
#[derive(Clone, Debug)]
pub struct GroupWrap<T> {
    delimiter: Delimiter,
    delim_span: Span,
    content_policy: ContentSpanPolicy,
    content: T,
}

impl<T: ToTokens> GroupWrap<T> {
    /// Wrap `content` so that interpolating the wrapper emits it inside a new
    /// group with the given `delimiter`.
    ///
    /// The delimiter of the new group is spanned with the synthetic
    /// `delim_span`, while the content tokens are treated according to
    /// `content_policy`.
    pub fn new(
        delimiter: Delimiter,
        delim_span: Span,
        content_policy: ContentSpanPolicy,
        content: T,
    ) -> Self {
        GroupWrap {
            delimiter,
            delim_span,
            content_policy,
            content,
        }
    }

    /// The delimiter of the group that the wrapper emits around the content.
    pub fn delimiter(&self) -> Delimiter {
        self.delimiter
    }

    /// The synthetic span applied to the delimiter of the new outer group.
    pub fn delim_span(&self) -> Span {
        self.delim_span
    }

    /// The span policy applied to the content tokens.
    pub fn content_span_policy(&self) -> ContentSpanPolicy {
        self.content_policy
    }
}

impl<T: ToTokens> ToTokens for GroupWrap<T> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let mut inner = TokenStream::new();
        match self.content_policy {
            // The content token stream is emitted exactly as produced by the
            // wrapped fragment: existing groups keep their delimiter, `None`
            // groups stay transparent, and interpolated variables keep the
            // spans coming from their own `ToTokens` implementation.
            ContentSpanPolicy::Preserve => self.content.to_tokens(&mut inner),
        }
        // `Group::set_span` only respans the delimiter of the new outer
        // group; it does not touch the content tokens. For `Delimiter::None`
        // the resulting group keeps a transparent boundary.
        let mut group = Group::new(self.delimiter, inner);
        group.set_span(self.delim_span);
        tokens.append(group);
    }
}
