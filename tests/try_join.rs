//! Acceptance tests for the feature-gated fallible collection joining
//! helpers (`fallible` feature):
//!
//! - empty / non-empty / failing fragment count combinations;
//! - the exact punctuation tree of the produced stream (including separator
//!   placement, spacing and group delimiters);
//! - the failing element index and its cause;
//! - zero calls into every fragment after the first failure;
//! - zero half-finished output on failure (transactional append and discarded
//!   staging, including tokens emitted by the failing fragment itself).
//!
//! All fixtures are in-memory (`proc_macro2::TokenStream` plus a small call
//! counter); no files or heavyweight infrastructure are involved.

use proc_macro2::TokenTree;
use proc_macro2::{Delimiter, Group, Ident, Punct, Spacing, Span, TokenStream};
use quote::{try_join, try_join_separated, ToTokens, TokenStreamExt, TryToTokens, TryTokenStreamExt};
use std::cell::Cell;
use std::fmt::Write as _;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Succeeds and emits no tokens.
    Empty,
    /// Succeeds and emits one identifier token.
    NonEmpty,
    /// Emits some tokens first and then fails: those tokens must be discarded.
    FailAfterTokens,
    /// Fails without emitting anything.
    Fail,
}

struct Frag {
    kind: Kind,
    name: &'static str,
    calls: Rc<Cell<usize>>,
}

impl TryToTokens for Frag {
    type Error = String;

    fn try_to_tokens(&self, tokens: &mut TokenStream) -> Result<(), String> {
        self.calls.set(self.calls.get() + 1);
        match self.kind {
            Kind::Empty => Ok(()),
            Kind::NonEmpty => {
                tokens.append(Ident::new(self.name, Span::call_site()));
                Ok(())
            }
            Kind::FailAfterTokens => {
                tokens.append(Ident::new("LEAKED", Span::call_site()));
                Err(format!("{} boom after tokens", self.name))
            }
            Kind::Fail => Err(format!("{} boom", self.name)),
        }
    }
}

/// Builds fragments from a list of kinds. The returned vector owns the call
/// counters; `calls[i]` is a shared handle to the counter of element `i`.
struct Fixture {
    fragments: Vec<Frag>,
    calls: Vec<Rc<Cell<usize>>>,
}

impl Fixture {
    fn new(kinds: &[(Kind, &'static str)]) -> Self {
        let mut fragments = Vec::new();
        let mut calls = Vec::new();
        for (kind, name) in kinds.iter().copied() {
            let counter = Rc::new(Cell::new(0usize));
            fragments.push(Frag {
                kind,
                name,
                calls: Rc::clone(&counter),
            });
            calls.push(counter);
        }
        Fixture { fragments, calls }
    }

    /// Moves the fragments out; counter handles stay usable for assertions.
    fn items(&mut self) -> Vec<Frag> {
        self.fragments.drain(..).collect()
    }

    fn counts(&self) -> Vec<usize> {
        self.calls.iter().map(|c| c.get()).collect()
    }
}

/// Renders a token stream as a depth-first punctuation tree: one leaf per
/// line, indented by nesting depth. Groups show their delimiter and
/// punctuation shows its spacing, so separator structure is fully visible.
fn dump_tree(stream: &TokenStream) -> String {
    let mut out = String::new();
    dump(&mut out, stream, 0);
    out
}

fn dump(out: &mut String, stream: &TokenStream, depth: usize) {
    for tt in stream.clone() {
        for _ in 0..depth {
            out.push_str("  ");
        }
        match tt {
            TokenTree::Group(group) => {
                let delim = match group.delimiter() {
                    Delimiter::Parenthesis => "Paren",
                    Delimiter::Brace => "Brace",
                    Delimiter::Bracket => "Bracket",
                    Delimiter::None => "None",
                };
                let _ = writeln!(out, "Group({delim})");
                dump(out, &group.stream(), depth + 1);
            }
            TokenTree::Ident(ident) => {
                let _ = writeln!(out, "Ident({})", ident);
            }
            TokenTree::Punct(punct) => {
                let spacing = match punct.spacing() {
                    Spacing::Alone => "Alone",
                    Spacing::Joint => "Joint",
                };
                let _ = writeln!(out, "Punct('{}', {spacing})", punct.as_char());
            }
            TokenTree::Literal(literal) => {
                let _ = writeln!(out, "Literal({})", literal);
            }
        }
    }
}

// Fragments consisting solely of empty, succeeding elements.
#[test]
fn all_empty_fragments() {
    let mut fx = Fixture::new(&[
        (Kind::Empty, "a"),
        (Kind::Empty, "b"),
        (Kind::Empty, "c"),
    ]);

    let result = try_join_separated(fx.items(), Punct::new(',', Spacing::Alone));

    let tokens = result.expect("all fragments succeed");
    assert!(tokens.is_empty(), "no tokens expected, got {tokens}");
    assert_eq!(fx.counts(), vec![1, 1, 1], "every fragment visited once");
    eprintln!("[all_empty_fragments] tree:\n{}", dump_tree(&tokens));
}

// Empty fragments must never attract separators.
#[test]
fn empties_interspersed_get_no_separators() {
    let mut fx = Fixture::new(&[
        (Kind::Empty, "a"),
        (Kind::NonEmpty, "f0"),
        (Kind::Empty, "b"),
        (Kind::Empty, "c"),
        (Kind::NonEmpty, "f1"),
        (Kind::Empty, "d"),
    ]);

    let result = try_join_separated(fx.items(), Punct::new(',', Spacing::Alone));

    let tokens = result.expect("fragments succeed");
    assert_eq!(tokens.to_string(), "f0 , f1");
    let tree = dump_tree(&tokens);
    assert_eq!(
        tree,
        concat!(
            "Ident(f0)\n",
            "Punct(',', Alone)\n",
            "Ident(f1)\n",
        ),
        "exactly one separator, between the two non-empty fragments:\n{tree}",
    );
    eprintln!("[empties_interspersed_get_no_separators] tree:\n{tree}");
}

// Without a separator, successful fragments concatenate directly and empties
// are skipped.
#[test]
fn join_without_separator() {
    let mut fx = Fixture::new(&[
        (Kind::NonEmpty, "a"),
        (Kind::Empty, "e"),
        (Kind::NonEmpty, "b"),
        (Kind::NonEmpty, "c"),
    ]);

    let result = try_join(fx.items());

    let tokens = result.expect("fragments succeed");
    assert_eq!(tokens.to_string(), "a b c");
    eprintln!(
        "[join_without_separator] tree:\n{}",
        dump_tree(&tokens),
    );
}

// First fragment fails: index 0, cause preserved, nothing produced, and no
// later fragment is ever called.
#[test]
fn first_failure_short_circuits() {
    let mut fx = Fixture::new(&[
        (Kind::Fail, "f0"),
        (Kind::NonEmpty, "f1"),
        (Kind::NonEmpty, "f2"),
    ]);

    let result = try_join_separated(fx.items(), Punct::new(',', Spacing::Alone));

    let error = result.expect_err("must fail");
    assert_eq!(error.index(), 0);
    assert_eq!(error.cause(), "f0 boom");
    let (index, cause): (usize, String) = error.into();
    assert_eq!(index, 0);
    assert_eq!(cause, "f0 boom");
    assert_eq!(fx.counts(), vec![1, 0, 0], "later fragments never called");
    eprintln!("[first_failure_short_circuits] counts={:?}", fx.counts());
}

// Failure after some successful non-empty fragments: the failing index and
// cause are reported; fragments after it get zero calls; and the function
// returns no half-finished TokenStream (its Ok payload cannot exist).
#[test]
fn later_failure_short_circuits_with_no_partial_output() {
    let mut fx = Fixture::new(&[
        (Kind::NonEmpty, "f0"),
        (Kind::Empty, "f1"),
        (Kind::NonEmpty, "f2"),
        (Kind::FailAfterTokens, "f3"),
        (Kind::NonEmpty, "f4"),
        (Kind::Fail, "f5"),
    ]);

    let result: Result<TokenStream, _> = try_join_separated(fx.items(), Punct::new(',', Spacing::Alone));

    assert!(result.is_err(), "no TokenStream is produced on failure");
    let error = result.unwrap_err();
    assert_eq!(error.index(), 3);
    assert_eq!(error.cause(), "f3 boom after tokens");
    assert_eq!(
        fx.counts(),
        vec![1, 1, 1, 1, 0, 0],
        "iteration stops at the first failing element",
    );
    eprintln!(
        "[later_failure_short_circuits_with_no_partial_output] \
         index={} cause={:?} counts={:?}",
        error.index(),
        error.cause(),
        fx.counts(),
    );
}

// Transactional append: pre-existing destination tokens survive a later
// failure untouched, and even the tokens of the failing fragment itself are
// not committed.
#[test]
fn failed_append_leaves_destination_untouched() {
    let mut fx = Fixture::new(&[
        (Kind::NonEmpty, "f0"),
        (Kind::FailAfterTokens, "f1"),
        (Kind::NonEmpty, "f2"),
    ]);
    let mut destination = quote_ident("KEEP");

    let error = destination
        .try_append_separated(fx.items(), Punct::new(',', Spacing::Alone))
        .expect_err("must fail");

    assert_eq!(error.index(), 1);
    assert_eq!(
        destination.to_string(),
        "KEEP",
        "destination must not gain any separator or partial fragment",
    );
    assert!(!destination.to_string().contains("LEAKED"));
    assert_eq!(fx.counts(), vec![1, 1, 0]);
    eprintln!(
        "[failed_append_leaves_destination_untouched] destination={:?} counts={:?}",
        destination.to_string(),
        fx.counts(),
    );
}

// A successful transactional append keeps the pre-existing content and only
// separators between new non-empty fragments are emitted (the pre-existing
// stream and the first new fragment are not separated).
#[test]
fn successful_appends_commit_after_existing_stream() {
    let mut fx = Fixture::new(&[
        (Kind::Empty, "e"),
        (Kind::NonEmpty, "f0"),
        (Kind::Empty, "e"),
        (Kind::NonEmpty, "f1"),
    ]);
    let mut destination = quote_ident("KEEP");

    destination
        .try_append_separated(fx.items(), Punct::new(',', Spacing::Alone))
        .expect("must succeed");

    assert_eq!(destination.to_string(), "KEEP f0 , f1");
    assert_eq!(fx.counts(), vec![1, 1, 1, 1]);
    eprintln!(
        "[successful_appends_commit_after_existing_stream] tree:\n{}",
        dump_tree(&destination),
    );
}

// Infallible ToTokens values join directly via the blanket implementation.
#[test]
fn infallible_to_tokens_values_join() {
    let numbers = vec![1u32, 2u32, 3u32];
    let tokens = try_join_separated(numbers, Punct::new(',', Spacing::Alone))
        .expect("Infallible never errors");
    assert_eq!(tokens.to_string(), "1u32 , 2u32 , 3u32");

    // Option<T: ToTokens> counts as an empty fragment when None.
    let mixed: Vec<Option<u32>> = vec![Some(10), None, Some(20), None];
    let tokens = try_join_separated(mixed, Punct::new(',', Spacing::Alone))
        .expect("infallible");
    assert_eq!(tokens.to_string(), "10u32 , 20u32");
    eprintln!(
        "[infallible_to_tokens_values_join] tree:\n{}",
        dump_tree(&tokens),
    );
}

// A multi-character separator is emitted with correct joint/alone spacing,
// and group tokens survive the join unchanged.
#[test]
fn multi_token_separator_and_group_punctuation_tree() {
    let group = {
        let mut stream = TokenStream::new();
        stream.append(Ident::new("inside", Span::call_site()));
        Group::new(Delimiter::Bracket, stream)
    };
    let sep = colon2();
    let items: Vec<TokenStream> = vec![
        group.to_token_stream(),
        TokenStream::new(),
        Ident::new("tail", Span::call_site()).to_token_stream(),
    ];

    let tokens = try_join_separated(items, sep).expect("infallible");

    let tree = dump_tree(&tokens);
    assert_eq!(
        tree,
        concat!(
            "Group(Bracket)\n",
            "  Ident(inside)\n",
            "Punct(':', Joint)\n",
            "Punct(':', Alone)\n",
            "Ident(tail)\n",
        ),
        "exact punctuation tree:\n{tree}",
    );
    eprintln!("[multi_token_separator_and_group_punctuation_tree] tree:\n{tree}");
}

// `try_append_all` is the no-separator counterpart of `try_join`.
#[test]
fn try_append_all_joins_without_separator() {
    let mut fx = Fixture::new(&[
        (Kind::NonEmpty, "a"),
        (Kind::Empty, "x"),
        (Kind::NonEmpty, "b"),
    ]);
    let mut destination = TokenStream::new();

    destination.try_append_all(fx.items()).expect("ok");

    assert_eq!(destination.to_string(), "a b");
    assert_eq!(fx.counts(), vec![1, 1, 1]);
    eprintln!(
        "[try_append_all_joins_without_separator] tree:\n{}",
        dump_tree(&destination),
    );
}

fn quote_ident(name: &str) -> TokenStream {
    let mut stream = TokenStream::new();
    stream.append(Ident::new(name, Span::call_site()));
    stream
}

// `::` with the joint/alone spacing emitted by a `quote!` repetition.
fn colon2() -> TokenStream {
    let mut stream = TokenStream::new();
    stream.append(Punct::new(':', Spacing::Joint));
    stream.append(Punct::new(':', Spacing::Alone));
    stream
}
