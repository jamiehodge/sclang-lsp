//! Parse one file and print its syntax errors with source context.
//!
//! ```text
//! cargo run --example check -- path/to/File.sc
//! ```

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: check <file.sc>");
        std::process::exit(2);
    };
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            println!("{path}: cannot read ({e})");
            return;
        }
    };
    let parse = sclang_syntax::parse(&src);

    if parse.errors.is_empty() {
        println!("{path}: ok");
        return;
    }

    println!("{path}: {} error(s)", parse.errors.len());
    for e in parse.errors.iter().take(10) {
        let line_no = src[..e.start as usize].matches('\n').count() + 1;
        let line_start = src[..e.start as usize].rfind('\n').map_or(0, |i| i + 1);
        let col = e.start as usize - line_start + 1;
        let line = src.lines().nth(line_no - 1).unwrap_or("");
        println!("\n  {line_no}:{col}  {}", e.message);
        println!("  | {}", line.trim_end());
        println!("  | {}^", " ".repeat(col.saturating_sub(1)));
    }
}
