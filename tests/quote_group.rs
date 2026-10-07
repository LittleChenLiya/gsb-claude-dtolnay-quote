use proc_macro2::{Delimiter, Group, Span, TokenStream, TokenTree};
use quote::{quote, quote_group, ContentSpanPolicy, GroupWrapper, ToTokens};

// Span sources used by these tests: tokens parsed from source text carry real
// byte ranges (proc-macro2 `span-locations` dev feature), while tokens
// produced by `quote!` itself sit at `Span::call_site()`, i.e. 0..0.
fn parsed(source: &str) -> TokenStream {
    source.parse().unwrap()
}

fn only_token(stream: TokenStream) -> TokenTree {
    let mut iter = stream.into_iter();
    let token = iter.next().expect("expected exactly one token");
    assert!(iter.next().is_none(), "expected exactly one token");
    token
}

fn only_group(stream: TokenStream) -> Group {
    match only_token(stream) {
        TokenTree::Group(group) => group,
        other => panic!("expected a group, got {:?}", other),
    }
}

fn assert_same_tokens(a: &TokenStream, b: &TokenStream) {
    let a: Vec<TokenTree> = a.clone().into_iter().collect();
    let b: Vec<TokenTree> = b.clone().into_iter().collect();
    assert_eq!(a.len(), b.len(), "token count mismatch");
    for (a, b) in a.iter().zip(&b) {
        assert_eq!(a.to_string(), b.to_string());
        assert_eq!(a.span().byte_range(), b.span().byte_range());
        if let (TokenTree::Group(a), TokenTree::Group(b)) = (a, b) {
            assert_eq!(a.delimiter(), b.delimiter());
            assert_same_tokens(&a.stream(), &b.stream());
        }
    }
}

// A small fragment fixture with two span sources and an inner None group:
//
// - `var`: interpolated variable parsed from "user_supplied" (span source A);
// - `none_group`: interpolated `Delimiter::None` group around parsed `a + b`,
//   carrying the span of `(a + b)` (span source B);
// - `delim_span`: synthetic delimiter span for the wrapper (span source C).
fn fixture() -> (TokenStream, Group, Span) {
    let var = parsed("user_supplied");

    let parens = only_group(parsed("(a + b)"));
    let mut none_group = Group::new(Delimiter::None, parens.stream());
    none_group.set_span(parens.span());

    let delim_span = only_group(parsed("  [  ]")).span();

    (var, none_group, delim_span)
}

#[test]
fn wrap_preserves_inner_groups_and_variable_spans() {
    let (var, none_group, delim_span) = fixture();
    let none_range = none_group.span().byte_range();
    let delim_range = delim_span.byte_range();
    assert_ne!(delim_range, 0..0, "synthetic delimiter span");

    let wrapped = quote_group!(Delimiter::Parenthesis, delim_span=>
        let #var = { #none_group };
    );
    assert_eq!(wrapped.delimiter(), Delimiter::Parenthesis);
    assert_eq!(wrapped.delim_span().byte_range(), delim_range);
    assert_eq!(wrapped.content_span_policy(), ContentSpanPolicy::Preserve);

    // Emitting through ToTokens produces exactly one token: the new outer group.
    let outer = only_group(wrapped.to_token_stream());
    assert_eq!(outer.delimiter(), Delimiter::Parenthesis);
    assert_eq!(outer.span().byte_range(), delim_range);

    let content: Vec<TokenTree> = outer.stream().into_iter().collect();
    assert_eq!(content.len(), 5);

    // Tokens originating in the quote! invocation keep call_site spans.
    assert_eq!(content[0].to_string(), "let");
    assert_eq!(content[0].span().byte_range(), 0..0);
    assert_eq!(content[2].to_string(), "=");
    assert_eq!(content[2].span().byte_range(), 0..0);
    assert_eq!(content[4].to_string(), ";");
    assert_eq!(content[4].span().byte_range(), 0..0);

    // The interpolated variable keeps its own span (source A): the wrapper
    // does not overwrite inner variable sources.
    assert_eq!(content[1].to_string(), "user_supplied");
    assert_eq!(content[1].span().byte_range(), 0..13);

    // The inner group written in the fragment keeps its delimiter.
    let TokenTree::Group(braces) = &content[3] else {
        panic!("expected brace group, got {:?}", content[3]);
    };
    assert_eq!(braces.delimiter(), Delimiter::Brace);
    assert_eq!(braces.span().byte_range(), 0..0);

    // The interpolated None group keeps its transparent boundary and span.
    let inner = only_group(braces.stream());
    assert_eq!(inner.delimiter(), Delimiter::None);
    assert_eq!(inner.span().byte_range(), none_range);

    // Tokens inside the None group keep their parsed spans (source B).
    let leaf: Vec<TokenTree> = inner.stream().into_iter().collect();
    assert_eq!(leaf.len(), 3);
    assert_eq!(leaf[0].to_string(), "a");
    assert_eq!(leaf[0].span().byte_range(), 1..2);
    assert_eq!(leaf[1].to_string(), "+");
    assert_eq!(leaf[1].span().byte_range(), 3..4);
    assert_eq!(leaf[2].to_string(), "b");
    assert_eq!(leaf[2].span().byte_range(), 5..6);
}

#[test]
fn wrap_empty_fragment() {
    let delim_span = only_group(parsed("  (  )")).span();
    let wrapped = quote_group!(Delimiter::Brace, delim_span=>);

    let outer = only_group(wrapped.into_token_stream());
    assert_eq!(outer.delimiter(), Delimiter::Brace);
    assert_eq!(outer.span().byte_range(), delim_span.byte_range());
    assert!(outer.stream().is_empty());
}

#[test]
fn none_delimiter_wrapper_is_transparent() {
    let (var, _none_group, delim_span) = fixture();

    let wrapped = quote_group!(Delimiter::None, delim_span=> #var);
    let outer = only_group(wrapped.into_token_stream());
    assert_eq!(outer.delimiter(), Delimiter::None);
    assert_eq!(outer.span().byte_range(), delim_span.byte_range());

    let ident = only_token(outer.stream());
    assert_eq!(ident.to_string(), "user_supplied");
    assert_eq!(ident.span().byte_range(), 0..13);
}

#[test]
fn wrapper_from_existing_stream() {
    let (var, none_group, delim_span) = fixture();
    let content = quote!(let #var = { #none_group };);

    let wrapped = GroupWrapper::new(Delimiter::Bracket, delim_span, content.clone())
        .with_content_span_policy(ContentSpanPolicy::Preserve);
    assert_eq!(wrapped.content_span_policy(), ContentSpanPolicy::Preserve);
    assert_same_tokens(wrapped.content(), &content);

    let group = only_group(wrapped.to_token_stream());
    assert_eq!(group.delimiter(), Delimiter::Bracket);
    assert_eq!(group.span().byte_range(), delim_span.byte_range());
    assert_same_tokens(&group.stream(), &content);

    let group = GroupWrapper::new(Delimiter::Bracket, delim_span, content).into_group();
    assert_eq!(group.delimiter(), Delimiter::Bracket);
    assert_eq!(group.span().byte_range(), delim_span.byte_range());
}

// Short regression example for the original interface: the wrapper content
// goes through the same `quote!` data flow, so both results must line up.
#[test]
fn original_interface_regression() {
    let (var, none_group, delim_span) = fixture();

    // Original interface: plain quote! on the same fragment.
    let plain = quote!(let #var = { #none_group };);

    let wrapped = quote_group!(Delimiter::Parenthesis, delim_span=>
        let #var = { #none_group };
    );
    let outer = only_group(wrapped.to_token_stream());

    // The wrapper content is exactly what the original interface produces,
    // token for token, span for span.
    assert_eq!(outer.stream().to_string(), plain.to_string());
    assert_same_tokens(&outer.stream(), &plain);

    // Cross-check against a manually built group: same result.
    let mut manual = Group::new(Delimiter::Parenthesis, plain);
    manual.set_span(delim_span);
    let manual = TokenStream::from(TokenTree::Group(manual));
    assert_same_tokens(&wrapped.into_token_stream(), &manual);

    // ToTokens compatibility: the wrapper interpolates into later quote!.
    let (var, none_group, delim_span) = fixture();
    let wrapped = quote_group!(Delimiter::Brace, delim_span=>
        let #var = { #none_group };
    );
    let outer = quote!(fn f() #wrapped);
    assert_eq!(
        outer.to_string(),
        "fn f () { let user_supplied = { a + b } ; }"
    );
}
