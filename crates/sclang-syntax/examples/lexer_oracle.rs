//! Differential test of our lexer against SuperCollider's own `sc_lexer`.
//!
//! Upstream extracted their lexer into a standalone library with a JSON token
//! dumper (`langutils/sc_lexer/standalone`). That makes it usable as a *test
//! oracle* rather than a build dependency: we keep a pure-Rust lexer, and CI
//! proves it agrees with theirs.
//!
//! ```text
//! ./oracle/build-sc-lexer.sh                       # fetch + build their tool
//! cargo run --release --example lexer_oracle -- <sc_lexer_standalone> <dir>...
//! ```

use sclang_syntax::{tokenize, SyntaxKind};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Map an upstream `TokenType` name onto our [`SyntaxKind`].
///
/// Where upstream draws a finer distinction than we do, several of their kinds
/// map onto one of ours; those are noted rather than hidden, since they are
/// candidates for refining our own taxonomy.
fn map_kind(upstream: &str) -> Option<SyntaxKind> {
    use SyntaxKind::*;
    Some(match upstream {
        "Name" => Ident,
        "ClassName" => ClassName,
        "PrimitiveName" => PrimitiveName,

        "Integer" => Integer,
        "IntegerRadix" | "FloatRadix" => RadixInteger, // we do not split these
        "Hexidecimal" => HexInteger,
        "Float" | "FloatExponent" => Float, // nor these
        "Pi" => PiKw,
        // sc_lexer classifies `inf` as a float literal.
        "Inf" => Float,
        "Accidental" | "AccidentalCents" => Accidental,
        "SymbolSlash" | "SymbolQuote" => Symbol,
        "Ascii" => Char,
        "True" => TrueKw,
        "False" => FalseKw,
        "Nil" => NilKw,
        // One token per quoted segment, as we do.
        "StringLine" | "String" => String,

        "While" => WhileKw,
        "Var" => VarKw,
        "Arg" => ArgKw,
        "ClassVar" => ClassvarKw,
        "Const" => ConstKw,

        "OpenParen" => LParen,
        "OpenSquare" => LBracket,
        "OpenCurly" => LBrace,
        "BeginClosedFunction" => BeginClosedFunc,
        "CloseParen" => RParen,
        "CloseSquare" => RBracket,
        "CloseCurly" => RBrace,

        "SemiColon" | "Semicolon" => Semicolon,
        "Colon" => Colon,
        "Comma" => Comma,
        "EqualsSign" => Eq,
        "NonLocalReturn" => Caret,
        "BackTick" => Backtick,
        "Tilde" => Tilde,
        "Hash" => Hash,
        "Ellipsis" => Ellipsis,
        "Dot" => Dot,
        "DotDot" => DotDot,
        "CurryArg" => CurryArg,

        "Pipe" => Pipe,
        "ReadWriteVar" => ReadWriteVar,
        "Minus" => Minus,
        "Multiply" => Star,
        "Add" => Plus,
        "LessThan" => Lt,
        "GreaterThan" => Gt,
        "BinaryOperator" => BinOp,
        "KeywordBinaryOperator" => KeywordBinop,

        // We emit one token per whitespace run; upstream splits by character
        // class. Runs are coalesced before comparing.
        "Space" | "NewLine" | "Tab" => Whitespace,
        "Comment" => LineComment,
        "MultilineComment" => BlockComment,

        other if other.starts_with("Er") => Error,
        _ => return None,
    })
}

/// One token from the upstream dumper: a type name and a byte length.
struct Upstream {
    kind: std::string::String,
    len: u32,
}

/// Parse `<TypeName> <byte_length>` lines from the dumper.
fn parse_dump(out: &str) -> Vec<Upstream> {
    out.lines()
        .filter_map(|line| {
            let (kind, len) = line.rsplit_once(' ')?;
            Some(Upstream {
                kind: kind.to_string(),
                len: len.trim().parse().ok()?,
            })
        })
        .collect()
}

/// Collapse consecutive whitespace tokens into one, summing their lengths, so
/// the two taxonomies line up on the only axis where they deliberately differ:
/// upstream splits whitespace by character class, we emit one token per run.
fn coalesce_ws(toks: Vec<(SyntaxKind, u32)>) -> Vec<(SyntaxKind, u32)> {
    let mut out: Vec<(SyntaxKind, u32)> = Vec::with_capacity(toks.len());
    for (k, len) in toks {
        if k == SyntaxKind::Whitespace {
            if let Some(last) = out.last_mut() {
                if last.0 == SyntaxKind::Whitespace {
                    last.1 += len;
                    continue;
                }
            }
        }
        out.push((k, len));
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect(&p, out);
        } else if matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("sc") | Some("scd")
        ) {
            out.push(p);
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(tool) = args.next() else {
        eprintln!("usage: lexer_oracle <sc_lexer_standalone> <dir>...");
        std::process::exit(2);
    };
    let dirs: Vec<std::string::String> = args.collect();

    let mut files = Vec::new();
    for d in &dirs {
        collect(Path::new(d), &mut files);
    }
    files.sort();

    let mut compared = 0usize;
    let mut agreed = 0usize;
    let mut skipped = 0usize;
    let mut mismatches: BTreeMap<std::string::String, usize> = BTreeMap::new();
    let mut samples: Vec<std::string::String> = Vec::new();
    let mut unmapped: BTreeMap<std::string::String, usize> = BTreeMap::new();

    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            skipped += 1;
            continue;
        };
        let Ok(out) = Command::new(&tool).arg(path).output() else {
            skipped += 1;
            continue;
        };
        let dump = std::string::String::from_utf8_lossy(&out.stdout);
        let theirs_raw = parse_dump(&dump);
        if theirs_raw.is_empty() && !src.trim().is_empty() {
            skipped += 1;
            continue;
        }

        let mut theirs = Vec::with_capacity(theirs_raw.len());
        let mut ok = true;
        for t in &theirs_raw {
            match map_kind(&t.kind) {
                Some(k) => theirs.push((k, t.len)),
                None => {
                    *unmapped.entry(t.kind.clone()).or_default() += 1;
                    ok = false;
                }
            }
        }
        if !ok {
            skipped += 1;
            continue;
        }

        let ours: Vec<(SyntaxKind, u32)> = tokenize(&src)
            .into_iter()
            .map(|t| (t.kind, t.len()))
            .collect();
        let theirs = coalesce_ws(theirs);
        let ours_c = coalesce_ws(ours);

        compared += 1;
        if theirs == ours_c {
            agreed += 1;
            continue;
        }

        // Find the first divergence and record it.
        let at = theirs
            .iter()
            .zip(ours_c.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(theirs.len().min(ours_c.len()));
        let their_k = theirs
            .get(at)
            .map(|t| format!("{:?}", t.0))
            .unwrap_or_else(|| "<end>".into());
        let our_k = ours_c
            .get(at)
            .map(|t| format!("{:?}", t.0))
            .unwrap_or_else(|| "<end>".into());
        let key = format!("theirs={their_k} ours={our_k}");
        *mismatches.entry(key.clone()).or_default() += 1;
        if samples.len() < 12 {
            // Reconstruct the offset so the sample can quote the source.
            let off: u32 = ours_c.iter().take(at).map(|t| t.1).sum();
            let snippet: std::string::String = src[off as usize..]
                .chars()
                .take(28)
                .collect::<std::string::String>();
            samples.push(format!(
                "{}  token {at}: {key}  at {off}: {snippet:?}",
                path.file_name().unwrap().to_string_lossy()
            ));
        }
    }

    println!("files compared : {compared}");
    println!("skipped        : {skipped}");
    println!(
        "agreed exactly : {agreed} / {compared} ({:.2}%)",
        100.0 * agreed as f64 / compared.max(1) as f64
    );

    if !unmapped.is_empty() {
        println!("\nunmapped upstream token types:");
        for (k, n) in &unmapped {
            println!("  {n:>6}  {k}");
        }
    }
    if !mismatches.is_empty() {
        let mut m: Vec<_> = mismatches.into_iter().collect();
        m.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        println!("\nfirst divergence, by kind:");
        for (k, n) in m.iter().take(15) {
            println!("  {n:>5}  {k}");
        }
        println!("\nsamples:");
        for s in &samples {
            println!("  {s}");
        }
    }
}
