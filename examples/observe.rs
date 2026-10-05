//! Demonstrates the [`quote_observe!`] macro with three interpolations.
//!
//! This example shows how to use an observer to record the source id and
//! output token range of each interpolation fragment. One of the three
//! fragments produces zero tokens, but still receives a receipt.
//!
//! Run locally with:
//!
//! ```sh
//! cargo run --example observe
//! ```
//!
//! [`quote_observe!`]: quote::quote_observe

use proc_macro2::{Ident, Span, TokenStream};
use quote::observe::{FragmentStatus, RecordingObserver};
use quote::{quote_observe, ToTokens};

/// A `ToTokens` implementation that produces no tokens.
struct Empty;

impl ToTokens for Empty {
    fn to_tokens(&self, _tokens: &mut TokenStream) {
        // Intentionally left blank. The observer still records a receipt.
    }
}

fn main() {
    let mut observer = RecordingObserver::new();

    let first = Ident::new("first", Span::call_site());
    let second = Empty;
    let third = Ident::new("third", Span::call_site());

    let tokens = quote_observe! { &mut observer=>
        #first #second #third
    };

    println!("output: {tokens}");
    println!("fragments observed: {}", observer.fragments.len());

    for fragment in &observer.fragments {
        println!(
            "  id={} range={}..{} status={:?}",
            fragment.id.index(),
            fragment.range.start,
            fragment.range.end,
            fragment.status,
        );
    }

    // All three fragments completed; the empty one has an empty range.
    assert_eq!(observer.fragments.len(), 3);
    assert!(observer.all_complete());
    assert_eq!(observer.fragments[0].range, 0..1);
    assert_eq!(observer.fragments[1].range, 1..1); // empty output
    assert_eq!(observer.fragments[2].range, 1..2);
    assert_eq!(observer.fragments[1].status, FragmentStatus::Complete);

    println!("all assertions passed");
}
