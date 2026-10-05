//! Acceptance tests for the explicit `ToTokens` observer in `quote::observe`.
//!
//! Focused local command:
//!
//! ```console
//! cargo test --test observe
//! ```

use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, AssertUnwindSafe};

use proc_macro2::{Delimiter, Ident, Punct, Spacing, Span, TokenStream, TokenTree};
use quote::observe::{self, FragmentId, FragmentKind, Observer};
use quote::{quote, ToTokens, TokenStreamExt};

const ID_ALPHA: FragmentId = FragmentId::new(1);
const ID_EMPTY: FragmentId = FragmentId::new(2);
const ID_COUNTED: FragmentId = FragmentId::new(3);

/// Emits zero tokens: the receipt must still be recorded.
struct Empty;

impl ToTokens for Empty {
    fn to_tokens(&self, _tokens: &mut TokenStream) {}
}

/// Counts how many times its `ToTokens` implementation runs.
struct Counted<'a> {
    calls: &'a Cell<usize>,
}

impl ToTokens for Counted<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.calls.set(self.calls.get() + 1);
        tokens.append(Ident::new("beta", Span::call_site()));
        tokens.append(Ident::new("gamma", Span::call_site()));
    }
}

#[test]
fn three_fragments_one_empty_ranges_and_ids() {
    let observer = RefCell::new(Observer::new());
    let alpha = Ident::new("alpha", Span::call_site());
    let calls = Cell::new(0);
    let counted = Counted { calls: &calls };

    let alpha_t = observe::trace(ID_ALPHA, &alpha, &observer);
    let empty_t = observe::trace(ID_EMPTY, &Empty, &observer);
    let counted_t = observe::trace(ID_COUNTED, &counted, &observer);

    let tokens = quote!(#alpha_t #empty_t #counted_t);
    assert_eq!(tokens.to_string(), "alpha beta gamma");

    // Custom ToTokens is invoked exactly once.
    assert_eq!(calls.get(), 1);

    let observer = observer.borrow();
    assert!(observer.is_complete());
    let fragments = observer.fragments();
    assert_eq!(fragments.len(), 3);

    // Every fragment is bound to its source id with an exact token range;
    // no guessing from the final string.
    let alpha = observer.fragment(ID_ALPHA).unwrap();
    assert_eq!(alpha.kind, FragmentKind::Interpolation);
    assert_eq!(alpha.stream, 0);
    assert_eq!(alpha.range, 0..1);

    // Zero output still leaves a source receipt with an empty range.
    let empty = observer.fragment(ID_EMPTY).unwrap();
    assert_eq!(empty.kind, FragmentKind::Interpolation);
    assert_eq!(empty.range, 1..1);

    let counted = observer.fragment(ID_COUNTED).unwrap();
    assert_eq!(counted.kind, FragmentKind::Interpolation);
    assert_eq!(counted.range, 1..3);

    // The recorded ranges really do select each fragment's tokens.
    let flat: Vec<TokenTree> = tokens.into_iter().collect();
    assert_eq!(flat[alpha.range.start].to_string(), "alpha");
    assert_eq!(flat[counted.range.start].to_string(), "beta");
    assert_eq!(flat[counted.range.end - 1].to_string(), "gamma");
}

const ID_GROUP: FragmentId = FragmentId::new(10);
const ID_LEAF: FragmentId = FragmentId::new(11);
const ID_NESTED: FragmentId = FragmentId::new(12);

/// Builds `item leaf [deep]` while the enclosing group context is open, so
/// the leaf and the nested group are recorded in the group's inner stream.
struct MidContent<'a> {
    observer: &'a RefCell<Observer>,
}

impl ToTokens for MidContent<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let leaf = Ident::new("leaf", Span::call_site());
        let leaf_t = observe::trace(ID_LEAF, &leaf, self.observer);
        let deep = Ident::new("deep", Span::call_site());
        let nested_t = observe::trace_group(ID_NESTED, Delimiter::Bracket, &deep, self.observer);
        quote!(item #leaf_t #nested_t).to_tokens(tokens);
    }
}

#[test]
fn nested_group_is_locatable() {
    let observer = RefCell::new(Observer::new());
    let mid = MidContent {
        observer: &observer,
    };
    let group_t = observe::trace_group(ID_GROUP, Delimiter::Parenthesis, &mid, &observer);

    let tokens = quote!(before #group_t after);
    assert_eq!(tokens.to_string(), "before (item leaf [deep]) after");

    let observer = observer.borrow();
    assert!(observer.is_complete());

    // The group's own range is one token in the root stream...
    let group = observer.fragment(ID_GROUP).unwrap();
    assert_eq!(group.kind, FragmentKind::Group(Delimiter::Parenthesis));
    assert_eq!(group.stream, 0);
    assert_eq!(group.range, 1..2);

    // ...recorded separately from its inner contents.
    let inner = group.inner.clone().unwrap();
    assert_eq!(inner.range, 0..3); // item, leaf, [deep]

    // The leaf lives in the group's inner stream context.
    let leaf = observer.fragment(ID_LEAF).unwrap();
    assert_eq!(leaf.stream, inner.stream);
    assert_eq!(leaf.range, 1..2);

    // The nested group too, with its own inner range recorded separately.
    let nested = observer.fragment(ID_NESTED).unwrap();
    assert_eq!(nested.kind, FragmentKind::Group(Delimiter::Bracket));
    assert_eq!(nested.stream, inner.stream);
    assert_eq!(nested.range, 2..3);
    let nested_inner = nested.inner.clone().unwrap();
    assert_eq!(nested_inner.range, 0..1);

    // Locate everything structurally from the final stream, guided only by
    // the recorded ranges — never by the string.
    let top: Vec<TokenTree> = tokens.into_iter().collect();
    let group_token = match &top[group.range.start] {
        TokenTree::Group(group) => group,
        other => panic!("expected group, got {other:?}"),
    };
    assert_eq!(group_token.delimiter(), Delimiter::Parenthesis);

    let mid_tokens: Vec<TokenTree> = group_token.stream().into_iter().collect();
    assert_eq!(mid_tokens.len(), inner.range.end - inner.range.start);
    assert_eq!(mid_tokens[leaf.range.start].to_string(), "leaf");

    let nested_token = match &mid_tokens[nested.range.start] {
        TokenTree::Group(group) => group,
        other => panic!("expected nested group, got {other:?}"),
    };
    assert_eq!(nested_token.delimiter(), Delimiter::Bracket);
    let deep_tokens: Vec<TokenTree> = nested_token.stream().into_iter().collect();
    assert_eq!(
        deep_tokens.len(),
        nested_inner.range.end - nested_inner.range.start
    );
    assert_eq!(deep_tokens[nested_inner.range.start].to_string(), "deep");
}

/// Emits `->` as two puncts with explicit spacing.
struct Arrow;

impl ToTokens for Arrow {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.append(Punct::new('-', Spacing::Joint));
        tokens.append(Punct::new('>', Spacing::Alone));
    }
}

#[test]
fn punct_spacing_is_preserved() {
    let observer = RefCell::new(Observer::new());
    let arrow_t = observe::trace(ID_ALPHA, &Arrow, &observer);

    let tokens = quote!(#arrow_t);
    assert_eq!(tokens.to_string(), "->");

    let flat: Vec<TokenTree> = tokens.into_iter().collect();
    assert_eq!(flat.len(), 2);
    match &flat[0] {
        TokenTree::Punct(punct) => {
            assert_eq!(punct.as_char(), '-');
            assert_eq!(punct.spacing(), Spacing::Joint);
        }
        other => panic!("expected punct, got {other:?}"),
    }
    match &flat[1] {
        TokenTree::Punct(punct) => {
            assert_eq!(punct.as_char(), '>');
            assert_eq!(punct.spacing(), Spacing::Alone);
        }
        other => panic!("expected punct, got {other:?}"),
    }

    let observer = observer.borrow();
    assert_eq!(observer.fragment(ID_ALPHA).unwrap().range, 0..2);
}

#[test]
fn default_to_tokens_is_unchanged() {
    let alpha = Ident::new("alpha", Span::call_site());
    let plain = quote!(#alpha).to_string();

    let observer = RefCell::new(Observer::new());
    let alpha_t = observe::trace(ID_ALPHA, &alpha, &observer);
    let observed = quote!(#alpha_t).to_string();

    assert_eq!(plain, observed);
    assert!(observer.borrow().is_complete());
}

#[test]
fn complete_observation_yields_stream() {
    let observation = observe::observe(|observer| {
        let name = Ident::new("answer", Span::call_site());
        let name_t = observe::trace(ID_ALPHA, &name, observer);
        quote!(let #name_t = 42;)
    });

    assert!(observation.is_complete());
    assert_eq!(observation.fragments().len(), 1);
    let stream = observation.into_stream().unwrap();
    assert_eq!(stream.to_string(), "let answer = 42 ;");
}

/// A `ToTokens` implementation that fails midway.
struct Panics;

impl ToTokens for Panics {
    fn to_tokens(&self, _tokens: &mut TokenStream) {
        panic!("boom");
    }
}

#[test]
fn failed_fragment_is_not_a_complete_stream() {
    let observation = observe::observe(|observer| {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let bad_t = observe::trace(ID_ALPHA, &Panics, observer);
            let _tokens = quote!(#bad_t);
        }));
        assert!(result.is_err());

        // The quotation can continue afterwards, but the failed fragment
        // must keep the observation from being called a complete stream.
        let ok = Ident::new("ok", Span::call_site());
        let ok_t = observe::trace(ID_EMPTY, &ok, observer);
        quote!(#ok_t)
    });

    assert!(!observation.is_complete());
    assert!(observation.fragments().iter().all(|f| f.id != ID_ALPHA));

    let incomplete = observation.into_stream().unwrap_err();
    let (partial, observer) = incomplete.into_parts();
    assert_eq!(partial.to_string(), "ok");
    assert!(observer.fragment(ID_EMPTY).is_some());
    assert!(!observer.is_complete());
}

#[test]
fn unfinished_fragment_is_not_complete() {
    let mut observer = Observer::new();
    observer.begin(ID_ALPHA, 0);
    // No matching end: the observation is unfinished.
    assert!(!observer.is_complete());
}
