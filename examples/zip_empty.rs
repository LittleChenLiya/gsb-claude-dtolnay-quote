//! Example 2: two empty sequences produce no items and, in particular, no
//! trailing separator.

use proc_macro2::Ident;
use quote::quote_zip;

fn main() {
    let params: Vec<Ident> = Vec::new();
    let tys: Vec<Ident> = Vec::new();

    let tokens = quote_zip! {
        params = params,
        tys = tys;
        fn f(#(#params: #tys),*) {}
    };

    println!("{}", tokens);
    assert_eq!(tokens.to_string(), "fn f () { }");
}
