//! Lexer tests.
//!
//! Two kinds live here. Most assert a specific token sequence for a specific
//! construct. A few assert *invariants* that must hold for any input at all —
//! those are the ones that catch the bugs nobody thought to write a case for.

use sclang_syntax::{tokenize, SyntaxKind, SyntaxKind::*, Token};

/// Token kinds, with trivia dropped. Convenient for asserting shape.
fn kinds(src: &str) -> Vec<sclang_syntax::SyntaxKind> {
    tokenize(src)
        .into_iter()
        .map(|t| t.kind)
        .filter(|k| !k.is_trivia())
        .collect()
}

/// `(kind, text)` pairs, trivia dropped.
fn pairs(src: &str) -> Vec<(sclang_syntax::SyntaxKind, &str)> {
    tokenize(src)
        .into_iter()
        .filter(|t| !t.kind.is_trivia())
        .map(|t| (t.kind, t.text(src)))
        .collect()
}

// =====================================================================
// Invariants — these must hold for every input.
// =====================================================================

/// Every byte of input is covered by exactly one token, with no gaps and no
/// overlaps. This is what makes the stream lossless, and it is the single most
/// valuable assertion in the file: a lexer that drops or double-counts a byte
/// will corrupt every downstream range.
fn assert_lossless(src: &str) {
    let tokens = tokenize(src);
    let mut offset = 0u32;
    for t in &tokens {
        assert_eq!(
            t.start, offset,
            "gap or overlap before {:?} in {src:?}",
            t.kind
        );
        assert!(t.end > t.start, "empty token {:?} in {src:?}", t.kind);
        offset = t.end;
    }
    assert_eq!(
        offset as usize,
        src.len(),
        "tokens do not reach end of {src:?}"
    );

    // Fully qualified: the glob import above brings `SyntaxKind::String` into
    // scope, which shadows the std type.
    let rebuilt: std::string::String = tokens.iter().map(|t| t.text(src)).collect();
    assert_eq!(rebuilt, src, "round-trip failed for {src:?}");
}

#[test]
fn lossless_over_a_representative_corpus() {
    let samples = [
        "",
        " ",
        "\n\n\t ",
        "Foo { bar { ^1 + 2 } }",
        "// just a comment",
        "/* nested /* comment */ still open */",
        "x = \"string with \\\" escape\";",
        "~env.value(\\sym, $a, 16rFF, 1e-8);",
        "SinOsc.ar(440) * 0.1",
        "#{ |a, b| a + b }",
        "(1, 3 .. 9)",
        "a[\\key] = [1, 2, 3];",
        "Array[slot] : ArrayedCollection { }",
        // Deliberately broken input: the lexer must still tile it completely.
        "\"unterminated",
        "/* unterminated",
        "1e",
        "€ £ ¥",
        "}}}]]]",
        "$",
    ];
    for s in samples {
        assert_lossless(s);
    }
}

#[test]
fn never_panics_on_arbitrary_bytes() {
    // Every ASCII character on its own, and a few multi-byte ones.
    for c in 0u8..=127 {
        let s = (c as char).to_string();
        assert_lossless(&s);
    }
    for s in ["\u{1F600}", "é", "日本語", "\u{0}"] {
        assert_lossless(s);
    }
}

/// `is_token` is a comparison against `Eof`'s position in the enum, so the
/// split between what the lexer produces and what the parser produces is held
/// by declaration order and by a comment. A kind appended after `Eof` — which
/// is where one naturally goes — would be called a node with nothing to say
/// so, and the two predicates would disagree with the thing they describe.
#[test]
fn every_kind_the_lexer_emits_is_a_token_kind() {
    // Between them these cover every variant `scan` can return.
    let corpus = concat!(
        "// comment\n/* block */ Foo : Bar { var <a, >b, <>c;\n",
        "  *new { |x = 1 ...rest| ^super.new(x, key: \\sym) }\n",
        "  ++ { arg y; _Prim_Thing; ^y.value(_) }\n",
        "}\n",
        "x = [1, 1.5, 16rFF, 0x1F, 1e-8, 4s, $c, 'quoted', \"str\", true, false, nil, pi];\n",
        "~e = (a: 1); y = #[1, 2]; z = #{ |q| q }; w = a[0..2]; v = `r; u = 2 <- 3;\n",
        "t = a <> b; s = -a + b * c / d % e; r = a ... b; q = a.b.(1); ?? \u{7}\n",
    );

    let mut seen = std::collections::BTreeSet::new();
    for token in tokenize(corpus) {
        assert!(
            token.kind.is_token(),
            "{:?} came out of the lexer but is_token() says it is a node",
            token.kind
        );
        assert!(!token.kind.is_node());
        seen.insert(format!("{:?}", token.kind));
    }

    // Enough of the set to make the assertion above worth something, and to
    // fail loudly if the corpus stops covering the interesting kinds.
    for kind in [
        "Whitespace",
        "LineComment",
        "BlockComment",
        "Integer",
        "Float",
        "RadixInteger",
        "HexInteger",
        "Accidental",
        "String",
        "Symbol",
        "Char",
        "Ident",
        "ClassName",
        "PrimitiveName",
        "CurryArg",
        "KeywordBinop",
        "VarKw",
        "ArgKw",
        "TrueKw",
        "FalseKw",
        "NilKw",
        "PiKw",
        "BinOp",
        "LeftArrow",
        "ReadWriteVar",
        "Pipe",
        "Lt",
        "Gt",
        "Minus",
        "Star",
        "Plus",
        "Eq",
        "LParen",
        "LBrace",
        "LBracket",
        "Semicolon",
        "Comma",
        "Dot",
        "DotDot",
        "Ellipsis",
        "Colon",
        "Caret",
        "Hash",
        "BeginClosedFunc",
        "Backtick",
        "Tilde",
        "Error",
    ] {
        assert!(
            seen.contains(kind),
            "{kind} is no longer covered by the corpus"
        );
    }

    // And the boundary itself, which nothing else would notice moving.
    assert!(SyntaxKind::Eof.is_token());
    assert!(SyntaxKind::SourceFile.is_node());
    assert!(SyntaxKind::ErrorNode.is_node());
}

// =====================================================================
// Trivia
// =====================================================================

#[test]
fn whitespace_is_one_token() {
    let t = tokenize("  \n\t x");
    assert_eq!(t[0].kind, Whitespace);
    assert_eq!(t[0].end, 5);
    assert_eq!(t[1].kind, Ident);
}

#[test]
fn line_comment_stops_before_newline() {
    let src = "// hi\nx";
    let t = tokenize(src);
    assert_eq!((t[0].kind, t[0].text(src)), (LineComment, "// hi"));
    assert_eq!(t[1].kind, Whitespace);
}

#[test]
fn block_comments_nest() {
    // The whole thing is one comment. A non-nesting lexer would stop at the
    // first `*/` and leave `still open */` as garbage.
    let src = "/* a /* b */ c */x";
    let t = tokenize(src);
    assert_eq!(
        (t[0].kind, t[0].text(src)),
        (BlockComment, "/* a /* b */ c */")
    );
    assert_eq!(t[1].kind, Ident);
}

#[test]
fn unterminated_block_comment_is_an_error_not_a_comment() {
    assert_eq!(kinds("/* open"), vec![Error]);
}

// =====================================================================
// Identifiers and keywords
// =====================================================================

#[test]
fn class_names_are_distinguished_from_identifiers() {
    assert_eq!(kinds("SinOsc foo"), vec![ClassName, Ident]);
}

#[test]
fn keywords() {
    assert_eq!(
        kinds("var arg classvar const while true false nil pi"),
        vec![VarKw, ArgKw, ClassvarKw, ConstKw, WhileKw, TrueKw, FalseKw, NilKw, PiKw]
    );
}

#[test]
fn this_is_just_an_identifier() {
    // lang11d declares a PSEUDOVAR token, but no lexer path returns it.
    assert_eq!(kinds("this thisProcess"), vec![Ident, Ident]);
}

#[test]
fn underscore_alone_is_curry_arg_but_underscore_name_is_a_primitive() {
    assert_eq!(kinds("_"), vec![CurryArg]);
    assert_eq!(kinds("_Symbol_envirPut"), vec![PrimitiveName]);
}

#[test]
fn identifier_followed_by_colon_is_one_keyword_binop_token() {
    // `s:5` must not lex as Ident, Colon, Integer.
    assert_eq!(pairs("s:5"), vec![(KeywordBinop, "s:"), (Integer, "5")]);
    assert_eq!(
        pairs("freq: 440"),
        vec![(KeywordBinop, "freq:"), (Integer, "440")]
    );
}

#[test]
fn a_bare_colon_is_still_a_colon() {
    // Class inheritance has no identifier immediately before the colon.
    assert_eq!(kinds("Foo : Bar"), vec![ClassName, Colon, ClassName]);
}

// =====================================================================
// Numbers
// =====================================================================

#[test]
fn integers_and_floats() {
    assert_eq!(pairs("42"), vec![(Integer, "42")]);
    assert_eq!(pairs("1.5"), vec![(Float, "1.5")]);
}

#[test]
fn dot_only_starts_a_float_when_a_digit_follows() {
    // The classic gotcha: `1.foo` is a method call on an integer.
    assert_eq!(
        pairs("1.foo"),
        vec![(Integer, "1"), (Dot, "."), (Ident, "foo")]
    );
    assert_eq!(pairs("1.5"), vec![(Float, "1.5")]);
    assert_eq!(
        pairs("2.rand"),
        vec![(Integer, "2"), (Dot, "."), (Ident, "rand")]
    );
}

#[test]
fn exponents() {
    assert_eq!(pairs("1e-8"), vec![(Float, "1e-8")]);
    assert_eq!(pairs("1e+8"), vec![(Float, "1e+8")]);
    assert_eq!(pairs("1e8"), vec![(Float, "1e8")]);
    assert_eq!(pairs("1.5e-3"), vec![(Float, "1.5e-3")]);
    // An exponent with no digits is an error in sclang too.
    assert_eq!(kinds("1e"), vec![Error]);
}

#[test]
fn radix_literals() {
    assert_eq!(pairs("16rF700"), vec![(RadixInteger, "16rF700")]);
    assert_eq!(pairs("2r1010"), vec![(RadixInteger, "2r1010")]);
    assert_eq!(pairs("16r1F.8"), vec![(RadixInteger, "16r1F.8")]);
}

#[test]
fn hex_literals() {
    assert_eq!(pairs("0x0002"), vec![(HexInteger, "0x0002")]);
    assert_eq!(pairs("0xFF"), vec![(HexInteger, "0xFF")]);
}

#[test]
fn accidentals() {
    // SuperCollider pitch notation, which is easy to miss entirely.
    assert_eq!(pairs("4s"), vec![(Accidental, "4s")]);
    assert_eq!(pairs("4ss"), vec![(Accidental, "4ss")]);
    assert_eq!(pairs("4s50"), vec![(Accidental, "4s50")]);
    assert_eq!(pairs("4b"), vec![(Accidental, "4b")]);
    assert_eq!(pairs("4bb"), vec![(Accidental, "4bb")]);
}

// =====================================================================
// Literals
// =====================================================================

#[test]
fn strings_handle_escaped_quotes() {
    let src = r#""a \" b""#;
    assert_eq!(pairs(src), vec![(String, r#""a \" b""#)]);
}

/// A fourth deliberate departure from sclang, and the same kind as the other
/// three: sclang's lexer exists to reject bad input, and an editor's has to
/// keep going.
///
/// An unterminated string or symbol is the ordinary state of a buffer someone
/// is typing in, and it runs to the end of the file by definition. Calling it
/// the literal it is lets the parser carry on with something it understands;
/// calling it an error derails everything after the quote. Either way the token
/// covers the same bytes, so nothing is lost.
#[test]
fn an_unterminated_literal_is_still_a_literal() {
    assert_eq!(kinds("\"open"), vec![String]);
    assert_eq!(kinds("'open"), vec![Symbol]);

    // A terminated one is unaffected, including across newlines.
    assert_eq!(pairs("\"a\nb\""), vec![(String, "\"a\nb\"")]);
}

#[test]
fn symbols_in_both_forms() {
    assert_eq!(pairs("\\foo"), vec![(Symbol, "\\foo")]);
    assert_eq!(pairs("'foo bar'"), vec![(Symbol, "'foo bar'")]);
    assert_eq!(pairs("\\123"), vec![(Symbol, "\\123")]);
    // A lone backslash is a valid empty symbol.
    assert_eq!(pairs("\\"), vec![(Symbol, "\\")]);
}

#[test]
fn characters() {
    assert_eq!(pairs("$a"), vec![(Char, "$a")]);
    assert_eq!(pairs("$\\n"), vec![(Char, "$\\n")]);
    // `$ ` is a space character literal, not an error.
    assert_eq!(pairs("$ "), vec![(Char, "$ ")]);
}

// =====================================================================
// Operators — the part that took longest to get right in the grammar.
// =====================================================================

#[test]
fn multi_character_operators_lex_as_one_token() {
    for op in [
        "++", "+++", "<!", "<<<", "@|@", "|@|", "<->", "**", "+/+", "&&", "==", "<<<*",
    ] {
        let src = format!("a {op} b");
        assert_eq!(
            pairs(&src),
            vec![(Ident, "a"), (BinOp, op), (Ident, "b")],
            "operator {op} did not lex as a single BinOp"
        );
    }
}

#[test]
fn reserved_single_characters_keep_their_own_kinds() {
    // These are separate tokens in sclang because the grammar needs them in
    // other roles (lang11d:10).
    assert_eq!(kinds("|"), vec![Pipe]);
    assert_eq!(kinds("<"), vec![Lt]);
    assert_eq!(kinds(">"), vec![Gt]);
    assert_eq!(kinds("-"), vec![Minus]);
    assert_eq!(kinds("*"), vec![Star]);
    assert_eq!(kinds("+"), vec![Plus]);
    assert_eq!(kinds("="), vec![Eq]);
}

#[test]
fn two_character_forms_that_are_not_generic_binops() {
    assert_eq!(kinds("<-"), vec![LeftArrow]);
    assert_eq!(kinds("<>"), vec![ReadWriteVar]);
}

#[test]
fn division_is_not_a_comment() {
    assert_eq!(
        pairs("1 / 2"),
        vec![(Integer, "1"), (BinOp, "/"), (Integer, "2")]
    );
}

#[test]
fn comment_wins_over_operator_at_the_same_position() {
    // `//` is two binop chars; it must still be a comment.
    let t = tokenize("1 // two");
    assert_eq!(t.last().unwrap().kind, LineComment);
}

#[test]
fn operator_runs_are_maximal() {
    // `a --- b` is one three-character operator, not `-` `-` `-`.
    assert_eq!(
        pairs("a --- b"),
        vec![(Ident, "a"), (BinOp, "---"), (Ident, "b")]
    );
}

// =====================================================================
// Punctuation and structure
// =====================================================================

#[test]
fn dot_forms() {
    assert_eq!(kinds("."), vec![Dot]);
    assert_eq!(kinds(".."), vec![DotDot]);
    assert_eq!(kinds("..."), vec![Ellipsis]);
    assert_eq!(
        kinds("(1..9)"),
        vec![LParen, Integer, DotDot, Integer, RParen]
    );
}

#[test]
fn hash_forms() {
    assert_eq!(kinds("#{"), vec![BeginClosedFunc]);
    assert_eq!(kinds("#["), vec![Hash, LBracket]);
    assert_eq!(kinds("#a"), vec![Hash, Ident]);
}

#[test]
fn environment_variable_prefix() {
    assert_eq!(pairs("~myvar"), vec![(Tilde, "~"), (Ident, "myvar")]);
}

// =====================================================================
// Whole constructs
// =====================================================================

#[test]
fn a_class_definition() {
    assert_eq!(
        kinds("Foo : Bar { *new { |a| ^super.new } }"),
        vec![
            ClassName, Colon, ClassName, LBrace, Star, Ident, LBrace, Pipe, Ident, Pipe, Caret,
            Ident, Dot, Ident, RBrace, RBrace
        ]
    );
}

#[test]
fn an_operator_method_definition() {
    assert_eq!(
        kinds("Foo { ++ { arg x; ^1 } }"),
        vec![
            ClassName, LBrace, BinOp, LBrace, ArgKw, Ident, Semicolon, Caret, Integer, RBrace,
            RBrace
        ]
    );
}

#[test]
fn getter_and_setter_markers() {
    assert_eq!(
        kinds("Foo { var <bar, >baz, <>qux; }"),
        vec![
            ClassName,
            LBrace,
            VarKw,
            Lt,
            Ident,
            Comma,
            Gt,
            Ident,
            Comma,
            ReadWriteVar,
            Ident,
            Semicolon,
            RBrace
        ]
    );
}

#[test]
fn token_ranges_are_usable_for_lsp_positions() {
    let src = "SinOsc.ar(440)";
    let t: Vec<Token> = tokenize(src);
    assert_eq!(t[0].text(src), "SinOsc");
    assert_eq!((t[0].start, t[0].end), (0, 6));
    let four_forty = t.iter().find(|t| t.kind == Integer).unwrap();
    assert_eq!(four_forty.text(src), "440");
    assert_eq!((four_forty.start, four_forty.end), (10, 13));
}

#[test]
fn line_comment_ends_at_a_carriage_return() {
    // Classic Mac line endings: stopping only at '\n' would make one comment
    // swallow the rest of the file (PyrLexer.cpp comment1 stops at both).
    let src = "// note\rx = 1;";
    let t = tokenize(src);
    assert_eq!((t[0].kind, t[0].text(src)), (LineComment, "// note"));
    assert_eq!(t[1].kind, Whitespace);
    assert_eq!(t[2].kind, Ident);
}

/// sclang splits non-ASCII two ways, and the split is observable: `x = [1,\u{a0}2]`
/// compiles, so a non-breaking space separates tokens; `var ±x = 1;` compiles
/// and `1 ± 2` does not, which is how an identifier behaves and not an
/// operator.
///
/// The help files are full of non-breaking spaces pasted in by accident, and
/// joining one to the next token turns a list into a syntax error.
#[test]
fn non_ascii_is_a_name_unless_it_is_a_space() {
    // Part of a name.
    assert_eq!(pairs("±"), vec![(Ident, "±")]);
    assert_eq!(pairs("±x"), vec![(Ident, "±x")]);
    assert_eq!(pairs("naïve"), vec![(Ident, "naïve")]);

    // A separator, like any other space: `pairs` drops trivia, so two names
    // coming back is exactly the point. `a±b` is one name for contrast.
    assert_eq!(pairs("a\u{a0}b"), vec![(Ident, "a"), (Ident, "b")]);
    assert_eq!(pairs("a±b"), vec![(Ident, "a±b")]);
}
