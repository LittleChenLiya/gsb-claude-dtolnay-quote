#![allow(clippy::disallowed_names)]

use proc_macro2::{Group, Ident, Spacing, Span, TokenStream, TokenTree};
use quote::observe::{FragmentStatus, RecordingObserver};
use quote::{quote, quote_observe, ToTokens, TokenStreamExt};
use std::cell::Cell;

/// A `ToTokens` implementation that counts how many times it is called.
struct Counting(Cell<usize>);

impl ToTokens for Counting {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.0.set(self.0.get() + 1);
        tokens.append(Ident::new("counted", Span::call_site()));
    }
}

#[test]
fn test_three_fragments_one_empty() {
    let mut observer = RecordingObserver::new();
    let a = Ident::new("alpha", Span::call_site());
    let b: Option<TokenStream> = None; // empty output
    let c = Ident::new("gamma", Span::call_site());
    let tokens = quote_observe! { &mut observer=> #a #b #c };

    assert_eq!(tokens.to_string(), "alpha gamma");
    assert_eq!(observer.fragments.len(), 3);
    assert!(observer.all_complete());

    // Fragment 0: alpha -> range 0..1
    let f0 = &observer.fragments[0];
    assert_eq!(f0.id.index(), 0);
    assert_eq!(f0.range, 0..1);
    assert_eq!(f0.status, FragmentStatus::Complete);
    assert!(f0.groups.is_empty());

    // Fragment 1: empty -> range 1..1 (zero output still has a receipt)
    let f1 = &observer.fragments[1];
    assert_eq!(f1.id.index(), 1);
    assert_eq!(f1.range, 1..1);
    assert_eq!(f1.status, FragmentStatus::Complete);
    assert!(f1.groups.is_empty());

    // Fragment 2: gamma -> range 1..2
    let f2 = &observer.fragments[2];
    assert_eq!(f2.id.index(), 2);
    assert_eq!(f2.range, 1..2);
    assert_eq!(f2.status, FragmentStatus::Complete);
    assert!(f2.groups.is_empty());
}

#[test]
fn test_counting_called_once() {
    let mut observer = RecordingObserver::new();
    let counter = Counting(Cell::new(0));
    let _tokens = quote_observe! { &mut observer=> #counter };

    // The custom ToTokens is called exactly once.
    assert_eq!(counter.0.get(), 1);
    assert_eq!(observer.fragments.len(), 1);
    assert!(observer.all_complete());
}

#[test]
fn test_nested_group_located() {
    let mut observer = RecordingObserver::new();
    let nested = quote! { a (b (c) d) e };
    let tokens = quote_observe! { &mut observer=> #nested };

    assert_eq!(tokens.to_string(), "a (b (c) d) e");
    assert_eq!(observer.fragments.len(), 1);
    assert!(observer.all_complete());

    let f = &observer.fragments[0];
    assert_eq!(f.id.index(), 0);
    // a, (b (c) d), e -> 3 top-level tokens
    assert_eq!(f.range, 0..3);
    assert_eq!(f.groups.len(), 1);

    let g = &f.groups[0];
    assert_eq!(g.offset, 1); // group sits at index 1 within the fragment
    assert_eq!(g.inner_len, 3); // b, (c), d
    assert_eq!(g.inner_groups.len(), 1);

    let ng = &g.inner_groups[0];
    assert_eq!(ng.offset, 1); // (c) sits at index 1 within the inner stream
    assert_eq!(ng.inner_len, 1); // c
    assert!(ng.inner_groups.is_empty());
}

#[test]
fn test_punct_spacing_preserved() {
    let mut observer = RecordingObserver::new();
    let tokens = quote_observe! { &mut observer=> :: -> => };

    // `::` is Joint, `->` is Joint, `=>` is Joint. The observer must not alter
    // the spacing of punctuation tokens.
    let mut iter = tokens.into_iter();
    let colon1 = iter.next().unwrap();
    let colon2 = iter.next().unwrap();
    assert_eq!(punct_char(&colon1), ':');
    assert_eq!(punct_char(&colon2), ':');
    assert_eq!(punct_spacing(&colon1), Spacing::Joint);
    assert_eq!(punct_spacing(&colon2), Spacing::Alone);

    let dash = iter.next().unwrap();
    let gt1 = iter.next().unwrap();
    assert_eq!(punct_char(&dash), '-');
    assert_eq!(punct_char(&gt1), '>');
    assert_eq!(punct_spacing(&dash), Spacing::Joint);
    assert_eq!(punct_spacing(&gt1), Spacing::Alone);

    let eq = iter.next().unwrap();
    let gt2 = iter.next().unwrap();
    assert_eq!(punct_char(&eq), '=');
    assert_eq!(punct_char(&gt2), '>');
    assert_eq!(punct_spacing(&eq), Spacing::Joint);
    assert_eq!(punct_spacing(&gt2), Spacing::Alone);

    assert!(iter.next().is_none());
    assert!(observer.all_complete());
}

fn punct_char(tt: &TokenTree) -> char {
    match tt {
        TokenTree::Punct(p) => p.as_char(),
        _ => panic!("expected punct"),
    }
}

fn punct_spacing(tt: &TokenTree) -> Spacing {
    match tt {
        TokenTree::Punct(p) => p.spacing(),
        _ => panic!("expected punct"),
    }
}

/// A `ToTokens` implementation that panics, to verify the incomplete state.
struct Panicking;

impl ToTokens for Panicking {
    fn to_tokens(&self, _tokens: &mut TokenStream) {
        panic!("intentional panic in to_tokens");
    }
}

#[test]
fn test_incomplete_not_complete() {
    let mut observer = RecordingObserver::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let p = Panicking;
        let _tokens = quote_observe! { &mut observer=> #p };
    }));

    assert!(result.is_err());
    assert_eq!(observer.fragments.len(), 1);
    let f = &observer.fragments[0];
    assert_eq!(f.status, FragmentStatus::Incomplete);
    // The failed fragment is not reported as a complete stream.
    assert!(!observer.all_complete());
}

#[test]
fn test_repetition_observed() {
    let mut observer = RecordingObserver::new();
    let items = vec![
        Ident::new("a", Span::call_site()),
        Ident::new("b", Span::call_site()),
        Ident::new("c", Span::call_site()),
    ];
    let tokens = quote_observe! { &mut observer=> #(#items),* };

    assert_eq!(tokens.to_string(), "a , b , c");
    // Each repetition iteration is observed separately.
    assert_eq!(observer.fragments.len(), 3);
    assert!(observer.all_complete());
    for (i, f) in observer.fragments.iter().enumerate() {
        assert_eq!(f.id.index(), i);
        assert_eq!(f.status, FragmentStatus::Complete);
    }
}

#[test]
fn test_group_produced_by_fragment() {
    let mut observer = RecordingObserver::new();
    let group = Group::new(
        proc_macro2::Delimiter::Parenthesis,
        quote! { x y },
    );
    let tokens = quote_observe! { &mut observer=> #group };

    assert_eq!(tokens.to_string(), "(x y)");
    assert_eq!(observer.fragments.len(), 1);
    let f = &observer.fragments[0];
    assert_eq!(f.range, 0..1);
    assert_eq!(f.groups.len(), 1);
    let g = &f.groups[0];
    assert_eq!(g.offset, 0);
    assert_eq!(g.inner_len, 2); // x, y
}

#[test]
fn test_default_quote_unchanged() {
    // The default `quote!` path must be unaffected by the observer feature.
    let tokens = quote! { a b c };
    assert_eq!(tokens.to_string(), "a b c");
}
