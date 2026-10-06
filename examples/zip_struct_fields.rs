//! Example 1: interpolate two length-3 sequences pairwise to generate the
//! fields of a struct, with a comma separator only between actual fields.

use quote::{format_ident, quote_zip};

fn main() {
    let fields = vec![
        format_ident!("width"),
        format_ident!("height"),
        format_ident!("depth"),
    ];
    let tys = vec![format_ident!("u32"), format_ident!("u32"), format_ident!("f64")];

    let tokens = quote_zip! {
        fields = fields,
        tys = tys;
        struct Dimensions {
            #(#fields: #tys),*
        }
    };

    println!("{}", tokens);
    assert_eq!(
        tokens.to_string(),
        "struct Dimensions { width : u32 , height : u32 , depth : f64 }",
    );
}
