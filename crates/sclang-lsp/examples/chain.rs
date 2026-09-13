//! Print a class's superclass chain, as the index sees it.
//!
//! A debugging aid. The chain is what decides which method a selector resolves
//! to, so when resolution looks wrong this is the first thing to check.
//!
//!     cargo run --release --example chain -- Pbind UGen AbstractFunction

use sclang_lsp::workspace::{build_index, default_roots};

fn main() {
    let (index, _r, _s) = build_index(&default_roots());
    for name in std::env::args().skip(1) {
        let class = index.class(&name);
        println!(
            "{name}: superclass={:?} file={:?}",
            class.map(|c| c.superclass.clone()),
            class.map(|c| c.location.file.file_name())
        );
        let chain: Vec<&str> = index
            .superclass_chain(&name)
            .into_iter()
            .map(|c| c.name.as_str())
            .collect();
        println!("  chain: {chain:?}");
    }
}
