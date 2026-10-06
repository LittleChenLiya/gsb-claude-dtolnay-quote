//! Example 3: sequences of different lengths (3 vs 2) are rejected with a
//! panic instead of being silently truncated like `zip` would.

use quote::{format_ident, quote_zip};

fn main() {
    let result = std::panic::catch_unwind(|| {
        let names = vec![format_ident!("a"), format_ident!("b"), format_ident!("c")];
        let values = vec![1i32, 2];

        quote_zip! {
            names = names,
            values = values;
            #(#names = #values),*
        }
    });

    match result {
        Ok(tokens) => println!("unexpected success: {}", tokens),
        Err(_) => println!("length mismatch was reported as an error"),
    }
}
