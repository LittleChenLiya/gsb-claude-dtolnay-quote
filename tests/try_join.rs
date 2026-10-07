use proc_macro2::{Ident, Punct, Spacing, Span, TokenStream, TokenTree};
use quote::{quote, try_join_tokens, JoinFailure, ToTokens, TokenStreamExt};
use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

/// A fragment that records how many times `to_tokens` was invoked and can be
/// configured to render as empty, as a single identifier, or to fail before
/// ever being tokenized.
struct CountingFragment {
    name: &'static str,
    kind: Kind,
    calls: Rc<Cell<usize>>,
}

#[derive(Clone, Copy)]
enum Kind {
    Empty,
    NonEmpty,
    Fail(&'static str),
}

impl CountingFragment {
    fn empty(name: &'static str, calls: &Rc<Cell<usize>>) -> Self {
        CountingFragment { name, kind: Kind::Empty, calls: Rc::clone(calls) }
    }

    fn non_empty(name: &'static str, calls: &Rc<Cell<usize>>) -> Self {
        CountingFragment { name, kind: Kind::NonEmpty, calls: Rc::clone(calls) }
    }

    fn fail(name: &'static str, cause: &'static str, calls: &Rc<Cell<usize>>) -> Self {
        CountingFragment { name, kind: Kind::Fail(cause), calls: Rc::clone(calls) }
    }

    fn into_result(self) -> Result<Self, String> {
        match self.kind {
            Kind::Fail(cause) => Err(cause.to_owned()),
            _ => Ok(self),
        }
    }
}

impl ToTokens for CountingFragment {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.calls.set(self.calls.get() + 1);
        if let Kind::NonEmpty = self.kind {
            tokens.append(Ident::new(self.name, Span::call_site()));
        }
    }
}

fn comma() -> TokenStream {
    quote!(,)
}

/// Render the token tree in a stable, punctuation-explicit form so tests can
/// assert on the exact tree shape, e.g. `Ident(a) Punct(',', Alone) Ident(b)`.
fn describe_tree(tokens: &TokenStream) -> String {
    let mut out = String::new();
    for tt in tokens.clone() {
        if !out.is_empty() {
            out.push(' ');
        }
        match tt {
            TokenTree::Ident(ident) => out.push_str(&format!("Ident({})", ident)),
            TokenTree::Punct(punct) => out.push_str(&format!(
                "Punct('{}', {:?})",
                punct.as_char(),
                punct.spacing()
            )),
            other => out.push_str(&other.to_string()),
        }
    }
    out
}

fn puncts(tokens: &TokenStream) -> Vec<char> {
    tokens
        .clone()
        .into_iter()
        .filter_map(|tt| match tt {
            TokenTree::Punct(p) => Some(p.as_char()),
            _ => None,
        })
        .collect()
}

#[test]
fn empty_input_yields_empty_stream() {
    let fragments: Vec<Result<CountingFragment, String>> = Vec::new();
    let joined = try_join_tokens(fragments, comma()).unwrap();
    assert!(joined.is_empty());
    assert_eq!(describe_tree(&joined), "");
}

#[test]
fn all_empty_fragments_produce_no_separator() {
    let calls = vec![Rc::new(Cell::new(0usize)), Rc::new(Cell::new(0))];
    let fragments: Vec<Result<CountingFragment, String>> = vec![
        Ok(CountingFragment::empty("a", &calls[0])),
        Ok(CountingFragment::empty("b", &calls[1])),
    ];
    let joined = try_join_tokens(fragments, comma()).unwrap();
    assert!(joined.is_empty());
    // Both fragments were consulted, but the separator never appears.
    assert_eq!(calls[0].get(), 1);
    assert_eq!(calls[1].get(), 1);
}

#[test]
fn non_empty_fragments_are_separated() {
    let calls: Vec<Rc<Cell<usize>>> = (0..3).map(|_| Rc::new(Cell::new(0))).collect();
    let fragments: Vec<Result<CountingFragment, String>> = vec![
        Ok(CountingFragment::non_empty("a", &calls[0])),
        Ok(CountingFragment::non_empty("b", &calls[1])),
        Ok(CountingFragment::non_empty("c", &calls[2])),
    ];
    let joined = try_join_tokens(fragments, comma()).unwrap();
    assert_eq!(joined.to_string(), "a , b , c");
    assert_eq!(puncts(&joined), vec![',', ',']);
    assert_eq!(
        describe_tree(&joined),
        "Ident(a) Punct(',', Alone) Ident(b) Punct(',', Alone) Ident(c)"
    );
    for counter in &calls {
        assert_eq!(counter.get(), 1);
    }
}

#[test]
fn separators_only_between_two_non_empty_fragments() {
    let calls: Vec<Rc<Cell<usize>>> = (0..5).map(|_| Rc::new(Cell::new(0))).collect();
    let fragments: Vec<Result<CountingFragment, String>> = vec![
        Ok(CountingFragment::empty("lead", &calls[0])),
        Ok(CountingFragment::non_empty("a", &calls[1])),
        Ok(CountingFragment::empty("mid", &calls[2])),
        Ok(CountingFragment::non_empty("b", &calls[3])),
        Ok(CountingFragment::empty("tail", &calls[4])),
    ];
    let joined = try_join_tokens(fragments, comma()).unwrap();
    // Exactly one comma, sitting between the only two non-empty fragments.
    assert_eq!(joined.to_string(), "a , b");
    assert_eq!(puncts(&joined), vec![',']);
    assert_eq!(describe_tree(&joined), "Ident(a) Punct(',', Alone) Ident(b)");
    // Every fragment (including the empty ones) was tokenized exactly once.
    for counter in &calls {
        assert_eq!(counter.get(), 1);
    }
}

#[test]
fn first_failure_short_circuits_with_index_and_cause() {
    let calls: Vec<Rc<Cell<usize>>> = (0..4).map(|_| Rc::new(Cell::new(0))).collect();
    let fragments = vec![
        CountingFragment::non_empty("a", &calls[0]).into_result(),
        CountingFragment::empty("skipped", &calls[1]).into_result(),
        CountingFragment::fail("boom", "boom cause", &calls[2]).into_result(),
        CountingFragment::non_empty("never", &calls[3]).into_result(),
    ];
    let result = try_join_tokens(fragments, comma());
    let failure: JoinFailure<String> = result.unwrap_err();
    // Failure reports the element index (counting empties) and the cause.
    assert_eq!(failure.index, 2);
    assert_eq!(failure.cause, "boom cause");
    assert_eq!(
        failure.to_string(),
        "fragment at index 2 failed to produce tokens: boom cause"
    );
    // Elements before the failure ran exactly once...
    assert_eq!(calls[0].get(), 1);
    assert_eq!(calls[1].get(), 1);
    // ...the failing element was never tokenized...
    assert_eq!(calls[2].get(), 0);
    // ...and everything after the failure saw zero calls.
    assert_eq!(calls[3].get(), 0);
}

#[test]
fn failure_at_first_element_touches_nothing_else() {
    let calls: Vec<Rc<Cell<usize>>> = (0..3).map(|_| Rc::new(Cell::new(0))).collect();
    let fragments = vec![
        CountingFragment::fail("x", "first", &calls[0]).into_result(),
        CountingFragment::non_empty("a", &calls[1]).into_result(),
        CountingFragment::non_empty("b", &calls[2]).into_result(),
    ];
    let failure = try_join_tokens(fragments, comma()).unwrap_err();
    assert_eq!(failure.index, 0);
    assert_eq!(failure.cause, "first");
    assert_eq!(calls[0].get(), 0);
    assert_eq!(calls[1].get(), 0);
    assert_eq!(calls[2].get(), 0);
}

#[test]
fn failure_returns_no_partial_token_stream() {
    let calls: Vec<Rc<Cell<usize>>> = (0..2).map(|_| Rc::new(Cell::new(0))).collect();
    let fragments = vec![
        CountingFragment::non_empty("a", &calls[0]).into_result(),
        CountingFragment::fail("boom", "kaboom", &calls[1]).into_result(),
    ];
    match try_join_tokens(fragments, comma()) {
        Ok(tokens) => panic!("expected failure, got tokens: {}", tokens),
        Err(failure) => {
            // The error carries only index + cause; the half-built stream
            // ("a" with no trailing separator) is dropped inside the helper.
            assert_eq!((failure.index, failure.cause.as_str()), (1, "kaboom"));
        }
    }
}

#[test]
fn failure_cause_is_mapped_without_losing_index() {
    let fragments: Vec<Result<CountingFragment, String>> =
        vec![CountingFragment::fail("x", "io", &Rc::new(Cell::new(0))).into_result()];
    let failure = try_join_tokens(fragments, comma())
        .map_err(|failure| failure.map_cause(|cause| cause.len()))
        .unwrap_err();
    assert_eq!(failure.index, 0);
    assert_eq!(failure.cause, 2);
}

#[test]
fn original_quote_interpolation_is_unchanged() {
    // The classic `#(...),*` repetition still emits the separator between
    // every repetition, including around empty interpolations.
    let names = vec!["a", "b"];
    let empties: Vec<TokenStream> = vec![TokenStream::new(), TokenStream::new()];
    let tokens = quote!(#(#names),* ; #(#empties),*);
    assert_eq!(tokens.to_string(), "\"a\" , \"b\" ; ,");

    // And a hand-rolled ToTokens impl still appends through the same trait.
    let mut stream = TokenStream::new();
    CountingFragment::non_empty("z", &Rc::new(Cell::new(0))).to_tokens(&mut stream);
    assert_eq!(stream.to_string(), "z");
}

/// Fixture: run one scenario of each class (empty / non-empty / failing),
/// write a human-readable report into a temp directory, read it back and
/// verify. The report path is printed so the run can be inspected.
#[test]
fn report_fixture_in_temp_dir() {
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "quote-try-join-report-{}-{}",
        std::process::id(),
        "fixture"
    ));
    fs::create_dir_all(&dir).unwrap();
    let report_path = dir.join("report.txt");

    // Empty class.
    let empty =
        try_join_tokens(Vec::<Result<CountingFragment, String>>::new(), comma()).unwrap();

    // Non-empty class with empty fragments mixed in.
    let mixed_fragments: Vec<Result<CountingFragment, String>> = vec![
        Ok(CountingFragment::empty("lead", &Rc::new(Cell::new(0)))),
        Ok(CountingFragment::non_empty("a", &Rc::new(Cell::new(0)))),
        Ok(CountingFragment::non_empty("b", &Rc::new(Cell::new(0)))),
    ];
    let mixed = try_join_tokens(mixed_fragments, comma()).unwrap();

    // Failing class.
    let tail_calls = Rc::new(Cell::new(0));
    let failure = try_join_tokens(
        vec![
            CountingFragment::non_empty("a", &Rc::new(Cell::new(0))).into_result(),
            CountingFragment::fail("boom", "disk full", &Rc::new(Cell::new(0))).into_result(),
            CountingFragment::non_empty("tail", &tail_calls).into_result(),
        ],
        comma(),
    )
    .unwrap_err();

    let report = format!(
        "empty.ok = {}\n\
         empty.tree = {:?}\n\
         mixed.tokens = {}\n\
         mixed.tree = {}\n\
         failure.index = {}\n\
         failure.cause = {}\n\
         failure.tail_calls = {}\n",
        empty.is_empty(),
        describe_tree(&empty),
        mixed,
        describe_tree(&mixed),
        failure.index,
        failure.cause,
        tail_calls.get(),
    );
    fs::write(&report_path, &report).unwrap();
    println!("report written to {}", report_path.display());
    print!("{}", report);

    let read_back = fs::read_to_string(&report_path).unwrap();
    assert_eq!(read_back, report);
    assert!(read_back.contains("mixed.tokens = a , b"));
    assert!(read_back.contains("failure.index = 1"));
    assert!(read_back.contains("failure.tail_calls = 0"));

    fs::remove_dir_all(&dir).unwrap();
}

/// Sanity check that a `Punct` separator other than a comma works too.
#[test]
fn custom_separator_tokens() {
    let plus = {
        let mut stream = TokenStream::new();
        stream.append(Punct::new('+', Spacing::Alone));
        stream
    };
    let fragments: Vec<Result<CountingFragment, String>> = vec![
        Ok(CountingFragment::non_empty("a", &Rc::new(Cell::new(0)))),
        Ok(CountingFragment::non_empty("b", &Rc::new(Cell::new(0)))),
    ];
    let joined = try_join_tokens(fragments, plus).unwrap();
    assert_eq!(joined.to_string(), "a + b");
    assert_eq!(puncts(&joined), vec!['+']);
}
