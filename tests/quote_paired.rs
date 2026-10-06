use proc_macro2::{Ident, Span};
use quote::{quote, quote_paired};

fn ident(name: &str) -> Ident {
    Ident::new(name, Span::call_site())
}

#[test]
fn paired_repetition_interleaves_in_order() {
    // Two sequences of length 3: items must be paired element by element.
    let names = vec![ident("a"), ident("b"), ident("c")];
    let values = vec![ident("x"), ident("y"), ident("z")];

    let tokens = quote_paired!((names, values) => #(#names = #values),*);

    // Separators appear only between actual items: no leading or trailing
    // comma.
    assert_eq!("a = x , b = y , c = z", tokens.to_string());
}

#[test]
fn paired_repetition_without_separator() {
    let names = vec![ident("a"), ident("b"), ident("c")];
    let values = vec![1u8, 2, 3];

    let tokens = quote_paired!((names, values) => #(let #names = #values;)*);

    assert_eq!(
        "let a = 1u8 ; let b = 2u8 ; let c = 3u8 ;",
        tokens.to_string()
    );
}

#[test]
#[should_panic(expected = "quote_paired! requires two sequences of the same length, got lengths 3 and 2")]
fn paired_repetition_rejects_mismatched_lengths() {
    // Length 3 vs length 2 must be an error, not a silent zip truncation.
    let names = vec![ident("a"), ident("b"), ident("c")];
    let values = vec![ident("x"), ident("y")];

    let _ = quote_paired!((names, values) => #(#names = #values),*);
}

#[test]
fn paired_repetition_empty_sequences_have_no_separator() {
    // Empty input: no items, so no separator and no trailing token.
    let names: Vec<Ident> = Vec::new();
    let values: Vec<Ident> = Vec::new();

    let tokens = quote_paired!((names, values) => #(#names = #values),*);

    assert_eq!("", tokens.to_string());
    assert!(tokens.is_empty());
}

#[test]
fn ordinary_quote_interpolation_is_unchanged() {
    // Plain interpolation and ordinary repetition through `quote!` behave
    // exactly as before.
    let x = ident("thing");
    assert_eq!("thing", quote!(#x).to_string());

    let xs = vec![ident("a"), ident("b")];
    assert_eq!("a , b", quote!(#(#xs),*).to_string());
    assert_eq!("a b", quote!(#(#xs)*).to_string());
    assert_eq!("a , b ,", quote!(#(#xs,)*).to_string());
}
