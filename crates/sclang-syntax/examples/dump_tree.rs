//! Print the concrete syntax tree for a snippet.
//!
//! A debugging aid, and the quickest way to find out what shape the parser
//! actually produces for something before writing code that matches on it.
//!
//!     cargo run --release --example dump_tree -- 'Pbind(\\dur, 1).play'

fn main() {
    let source = std::env::args().nth(1).unwrap_or_default();
    let parse = sclang_syntax::parse(&source);
    println!("{}", parse.root.debug_tree(&source));
    for e in &parse.errors {
        println!("error: {}", e.message);
    }
}
