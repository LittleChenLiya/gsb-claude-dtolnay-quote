//! Targeted tests for the `quote_zip!` equal-length repetition entry point.

use proc_macro2::{Ident, Span};
use quote::{quote, quote_zip};

fn idents(names: &[&str]) -> Vec<Ident> {
    names
        .iter()
        .map(|name| Ident::new(name, Span::call_site()))
        .collect()
}

#[test]
fn test_zip_equal_length_interleaves_items() {
    // Two sequences of length 3 interpolate item by item.
    let names = idents(&["a", "b", "c"]);
    let values = vec![1i32, 2, 3];

    let tokens = quote_zip! {
        names = names,
        values = values;
        #(let #names = #values;)*
    };

    assert_eq!(
        tokens.to_string(),
        "let a = 1i32 ; let b = 2i32 ; let c = 3i32 ;",
    );
}

#[test]
fn test_zip_separator_only_between_items() {
    // A separator is emitted only between actual items: exactly two commas
    // for three items, none leading or trailing.
    let names = idents(&["x", "y", "z"]);
    let values = vec![10i32, 20, 30];

    let tokens = quote_zip! {
        names = names,
        values = values;
        f(#(#names: #values),*)
    };

    assert_eq!(tokens.to_string(), "f (x : 10i32 , y : 20i32 , z : 30i32)");
}

#[test]
fn test_zip_empty_produces_no_separator() {
    // Empty sequences produce no output at all, in particular no trailing
    // separator.
    let names: Vec<Ident> = Vec::new();
    let values: Vec<i32> = Vec::new();

    let tokens = quote_zip! {
        names = names,
        values = values;
        f(#(#names = #values),*)
    };

    assert_eq!(tokens.to_string(), "f ()");
}

#[test]
fn test_zip_accepts_slices_and_arrays() {
    // Any replayable sequence with a definite length works, not just Vec.
    let names = idents(&["p", "q", "r"]);
    let values = [7i32, 8, 9];

    let tokens = quote_zip! {
        names = &names[..],
        values = values;
        #(#names = #values),*
    };

    assert_eq!(tokens.to_string(), "p = 7i32 , q = 8i32 , r = 9i32");
}

#[test]
#[should_panic(expected = "quote_zip! requires two sequences of equal length, got 3 and 2")]
fn test_zip_length_mismatch_is_an_error() {
    // Length 3 vs length 2 must be rejected, never silently truncated.
    let names = idents(&["a", "b", "c"]);
    let values = vec![1i32, 2];

    let _ = quote_zip! {
        names = names,
        values = values;
        #(#names = #values),*
    };
}

#[test]
#[should_panic(expected = "quote_zip! requires two sequences of equal length, got 0 and 1")]
fn test_zip_empty_against_nonempty_is_an_error() {
    let names: Vec<Ident> = Vec::new();
    let values = vec![1i32];

    let _ = quote_zip! {
        names = names,
        values = values;
        #(#names = #values),*
    };
}

#[test]
fn test_plain_quote_repetition_unchanged() {
    // The original `quote!` repetition keeps its existing behavior: it stops
    // at the shorter sequence without complaining.
    let names = idents(&["a", "b", "c"]);
    let values = vec![1i32, 2];

    let tokens = quote! {
        #(#names = #values),*
    };

    assert_eq!(tokens.to_string(), "a = 1i32 , b = 2i32");
}

#[test]
fn test_plain_quote_single_interpolation_unchanged() {
    let name = Ident::new("answer", Span::call_site());
    let value = 42i32;

    let tokens = quote! {
        let #name = #value;
    };

    assert_eq!(tokens.to_string(), "let answer = 42i32 ;");
}
