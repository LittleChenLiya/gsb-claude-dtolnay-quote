//! Small in-memory fixture demonstrating the `GroupWrap` span policy.
//!
//! Run with: cargo run --example group_wrap

use proc_macro2::{Delimiter, Group, Span, TokenStream, TokenTree};
use quote::{quote, ContentSpanPolicy, GroupWrap};

fn describe(span: Span) -> String {
    match span.source_text() {
        Some(text) => format!("source {:?}", text),
        None => "call_site".to_owned(),
    }
}

fn main() {
    // Two span sources: a parsed variable and a synthetic delimiter span.
    let var: TokenStream = "source_var".parse().unwrap();
    let synthetic_span = "synthetic_delim"
        .parse::<TokenStream>()
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .span();

    // Fragment with an existing group, a transparent None group, and the var.
    let transparent = Group::new(Delimiter::None, quote!(#var));
    let fragment = quote! {
        let x = (#var);
        #transparent
    };

    let wrapped = GroupWrap::new(
        Delimiter::Brace,
        synthetic_span,
        ContentSpanPolicy::Preserve,
        fragment,
    );
    let tokens = quote!(#wrapped);

    println!("tokens: {}", tokens);
    let outer = match tokens.into_iter().next().unwrap() {
        TokenTree::Group(group) => group,
        other => panic!("expected group, got {:?}", other),
    };
    println!(
        "outer group: delimiter={:?} delim_span={}",
        outer.delimiter(),
        describe(outer.span()),
    );
    for tt in outer.stream() {
        match tt {
            TokenTree::Group(group) => {
                println!(
                    "  inner group: delimiter={:?} delim_span={}",
                    group.delimiter(),
                    describe(group.span()),
                );
                for inner in group.stream() {
                    println!("    {:?} span={}", inner, describe(inner.span()));
                }
            }
            other => println!("  {:?} span={}", other, describe(other.span())),
        }
    }

    // Empty fragment wrapping.
    let empty = GroupWrap::new(
        Delimiter::Parenthesis,
        synthetic_span,
        ContentSpanPolicy::Preserve,
        TokenStream::new(),
    );
    let empty_tokens = quote!(#empty);
    println!("empty wrapped: {}", empty_tokens);
}
