use crate::ToTokens;
use proc_macro2::{Delimiter, Group, Span, TokenStream};

/// Policy describing how the spans of a fragment's content tokens are treated
/// when the fragment is wrapped in a synthetic nested group by
/// [`GroupWrapper`].
///
/// The delimiter span of the synthetic group is always specified separately
/// from this policy; see [`GroupWrapper::new`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ContentSpanPolicy {
    /// Keep the span of every content token exactly as produced by the
    /// fragment's own data flow: tokens written literally in a `quote!`
    /// invocation keep that invocation's spans, and interpolated `#var`
    /// tokens keep the spans coming from their own `ToTokens`
    /// implementation. The synthetic delimiter span is applied to the
    /// wrapping group only and never overwrites content spans.
    #[default]
    Preserve,
}

/// A quoted fragment wrapped in a synthetic nested [`Group`].
///
/// This is the return type of the [`quote_group!`] macro, and can also be
/// built directly from an existing `TokenStream` with [`GroupWrapper::new`].
///
/// [`quote_group!`]: crate::quote_group
///
/// The wrapper applies two span policies independently:
///
/// - the delimiter of the synthetic group receives the explicitly provided
///   synthetic span;
/// - the content tokens are treated according to a [`ContentSpanPolicy`],
///   which by default preserves their original spans, including the spans of
///   interpolated variables.
///
/// Groups already present inside the fragment keep their delimiter and spans;
/// in particular `Delimiter::None` groups keep their transparent boundary.
///
/// `GroupWrapper` implements [`ToTokens`], so a wrapped fragment can be
/// interpolated into further `quote!` invocations like any other value.
#[derive(Clone, Debug)]
pub struct GroupWrapper {
    delimiter: Delimiter,
    delim_span: Span,
    content_policy: ContentSpanPolicy,
    content: TokenStream,
}

impl GroupWrapper {
    /// Wraps `content` in a synthetic group with the given `delimiter`, whose
    /// span is `delim_span`. The content span policy defaults to
    /// [`ContentSpanPolicy::Preserve`].
    pub fn new(delimiter: Delimiter, delim_span: Span, content: TokenStream) -> Self {
        GroupWrapper {
            delimiter,
            delim_span,
            content_policy: ContentSpanPolicy::default(),
            content,
        }
    }

    /// Sets the policy applied to the spans of the content tokens.
    pub fn with_content_span_policy(mut self, policy: ContentSpanPolicy) -> Self {
        self.content_policy = policy;
        self
    }

    /// The delimiter of the synthetic wrapping group.
    pub fn delimiter(&self) -> Delimiter {
        self.delimiter
    }

    /// The synthetic span applied to the wrapping group's delimiter.
    pub fn delim_span(&self) -> Span {
        self.delim_span
    }

    /// The policy applied to the spans of the content tokens.
    pub fn content_span_policy(&self) -> ContentSpanPolicy {
        self.content_policy
    }

    /// The wrapped fragment, with its spans preserved.
    pub fn content(&self) -> &TokenStream {
        &self.content
    }

    /// Unwraps into the synthetic `Group` token: `delimiter` spanned with
    /// `delim_span`, content tokens untouched.
    pub fn into_group(self) -> Group {
        let mut group = Group::new(self.delimiter, self.content);
        group.set_span(self.delim_span);
        group
    }
}

impl ToTokens for GroupWrapper {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        // Same data flow as the nested groups produced by `quote_spanned!`:
        // the synthetic span lands on the wrapping group's delimiter only,
        // while the content tokens (including interpolated variables and
        // inner groups) keep their own spans and delimiters.
        crate::__private::push_group_spanned(
            tokens,
            self.delim_span,
            self.delimiter,
            self.content.clone(),
        );
    }
}
