//! Parser tests.
//!
//! As with the lexer, the invariants matter more than the individual cases:
//! the tree must always cover the source exactly, and parsing must never hang
//! or panic no matter how broken the input.

use sclang_syntax::{parse, Child, SyntaxKind, SyntaxNode};

/// Reconstruct the source from the tree's tokens.
fn reconstruct(node: &SyntaxNode, source: &str, out: &mut String) {
    for child in &node.children {
        match child {
            Child::Node(n) => reconstruct(n, source, out),
            Child::Token(t) => out.push_str(t.text(source)),
        }
    }
}

/// The tree must contain every byte of the source, in order, exactly once.
fn assert_lossless(src: &str) {
    let parse = parse(src);
    let mut rebuilt = String::new();
    reconstruct(&parse.root, src, &mut rebuilt);
    assert_eq!(rebuilt, src, "tree did not round-trip for {src:?}");
}

/// Every node's range must be a real span of the source, inside its parent's
/// and after its previous sibling's.
///
/// Losslessness does not imply this: the tokens can tile the input perfectly
/// while a node that ended up with no children claims the wrong range, and a
/// node whose last child is one of those inherits it. That is how a node
/// ending before it started reached the server, where reading its text
/// panicked — `Foo { var a = #; }` was enough.
fn assert_well_formed(src: &str) {
    fn walk(node: &SyntaxNode, src: &str, path: &str) {
        assert!(
            node.start <= node.end,
            "{path}: {:?} ends at {} before it starts at {} in {src:?}",
            node.kind,
            node.end,
            node.start
        );
        let mut previous = node.start;
        for (i, child) in node.children.iter().enumerate() {
            let (start, end) = child.range();
            let where_ = format!("{path}/{i}:{:?}", child.kind());
            assert!(
                start >= previous && end <= node.end,
                "{where_}: {start}..{end} escapes {:?} {}..{} in {src:?}",
                node.kind,
                node.start,
                node.end
            );
            previous = end;
            if let Child::Node(n) = child {
                walk(n, src, &where_);
            }
        }
    }

    let parse = parse(src);
    assert_eq!(
        (parse.root.start, parse.root.end),
        (0, src.len() as u32),
        "the root did not cover {src:?}"
    );
    walk(&parse.root, src, "root");
}

/// Recovery must not report the same thing over and over.
///
/// A recovery point the cursor cannot reach used to leave the class-body loop
/// asking from the same place until its fuel ran out, so one stray `)` became
/// 257 identical diagnostics in the editor. Two errors per token is the worst
/// any well-formed recovery produces; the bound here is loose enough not to
/// need revisiting and tight enough to catch a stall.
fn assert_no_error_flood(src: &str) {
    let parse = parse(src);
    let tokens = sclang_syntax::tokenize(src).len();
    assert!(
        parse.errors.len() <= 4 * tokens + 4,
        "{} errors for {tokens} tokens in {src:?}: {:?}",
        parse.errors.len(),
        parse.errors.first()
    );
}

fn tree(src: &str) -> String {
    parse(src).root.debug_tree(src)
}

/// Count nodes of a kind anywhere in the tree.
fn count(src: &str, kind: SyntaxKind) -> usize {
    parse(src)
        .root
        .descendants()
        .iter()
        .filter(|n| n.kind == kind)
        .count()
}

// =====================================================================
// Invariants
// =====================================================================

#[test]
fn trees_are_lossless() {
    for src in [
        "",
        "Foo { }",
        "Foo : Bar { baz { ^1 } }",
        "+ Object { foo { ^this } }",
        "x = SinOsc.ar(440, mul: 0.1);",
        "// leading comment\nFoo { /* inner */ bar { ^1 } }",
        "Array[slot] : ArrayedCollection { }",
        "f = { |a, b = 2| a + b };",
        "(1, 3 .. 9)",
        "~env = (freq: 440, dur: 1);",
        "#a, b = [1, 2];",
        "Foo { ++ { arg x; ^x } }",
        // Broken input must still round-trip.
        "Foo { bar { ^1 +",
        "Foo {{{{",
        "} } ] ) ;;;",
        "Foo { var ; }",
        "1 + + + 2",
    ] {
        assert_lossless(src);
    }
}

#[test]
fn never_hangs_on_pathological_input() {
    // Unbalanced delimiters in both directions, and deep nesting.
    for src in [
        "((((((((((",
        "))))))))))",
        "{".repeat(200).as_str(),
        "[,,,,,,]",
        "^^^^^",
    ] {
        let _ = parse(src); // must terminate
        assert_lossless(src);
    }
}

#[test]
fn node_ranges_are_well_formed() {
    for src in [
        "",
        "Foo { }",
        "Foo : Bar { baz { ^1 } }",
        "// leading comment\nFoo { /* inner */ bar { ^1 } }",
        // Each of these used to produce a node at `0..0`, or one ending
        // before it started.
        "Foo { var a = #; }",
        "Foo  var a = #; }",
        "aaa#",
        "x = [1, )];",
        "Foo { ) }",
        "Foo { bar { ^1 } ] }",
    ] {
        assert_well_formed(src);
    }
}

/// Every four-character string over the punctuation that drives the grammar.
/// Short enough to enumerate exhaustively, which beats sampling: the shapes
/// that break a parser are small and adjacent, and all three of the bugs this
/// guards against show up inside four characters.
#[test]
fn short_inputs_are_exhaustively_survivable() {
    let alphabet: Vec<char> = "aA1 {}()[];,:=|*#^.-<>".chars().collect();
    let mut buffer = String::new();
    for &a in &alphabet {
        for &b in &alphabet {
            for &c in &alphabet {
                for &d in &alphabet {
                    buffer.clear();
                    buffer.extend([a, b, c, d]);
                    assert_lossless(&buffer);
                    assert_well_formed(&buffer);
                    assert_no_error_flood(&buffer);
                }
            }
        }
    }
}

// =====================================================================
// Top level
// =====================================================================

#[test]
fn class_definition_with_superclass() {
    let t = tree("Foo : Bar { }");
    assert!(t.contains("ClassDef"), "{t}");
    assert!(t.contains("SuperClass"), "{t}");
}

#[test]
fn indexed_class_definition() {
    // `Array[slot] : ArrayedCollection` — the form that needs `[slot]` to not
    // be read as a collection literal.
    let t = tree("Array[slot] : ArrayedCollection { }");
    assert!(t.contains("ClassDef"), "{t}");
    assert!(t.contains("IndexedSlot"), "{t}");
    assert!(t.contains("SuperClass"), "{t}");
}

#[test]
fn class_extension() {
    let t = tree("+ Object { foo { ^1 } }");
    assert!(t.contains("ClassExtension"), "{t}");
    assert_eq!(count("+ Object { foo { ^1 } }", SyntaxKind::MethodDef), 1);
}

#[test]
fn class_name_followed_by_dot_is_an_expression_not_a_class_def() {
    let src = "SinOsc.ar(440)";
    assert_eq!(count(src, SyntaxKind::ClassDef), 0);
    assert_eq!(count(src, SyntaxKind::MethodCall), 1);
}

// =====================================================================
// Members
// =====================================================================

#[test]
fn instance_and_class_methods() {
    let src = "Foo { bar { ^1 } *baz { ^2 } }";
    assert_eq!(count(src, SyntaxKind::MethodDef), 2);
}

#[test]
fn operator_named_methods() {
    // lang11d:107 — `binop '{' ... '}'`
    for src in [
        "Foo { ++ { ^1 } }",
        "Foo { < { ^1 } }",
        "Foo { <> { ^1 } }",
        "Foo { * { ^1 } }",
    ] {
        assert_eq!(count(src, SyntaxKind::MethodDef), 1, "failed for {src}");
    }
}

#[test]
fn declarations_with_getter_setter_markers() {
    let src = "Foo { var <bar, >baz, <>qux; classvar count = 0; }";
    assert_eq!(count(src, SyntaxKind::ClassVarDecl), 2);
    assert_eq!(count(src, SyntaxKind::SlotDef), 4);
    assert_eq!(count(src, SyntaxKind::RwSpec), 3);
}

#[test]
fn primitive_in_method_body() {
    let src = "Foo { bar { _Symbol_envirPut; ^this.primitiveFailed } }";
    assert_eq!(count(src, SyntaxKind::Primitive), 1);
}

// =====================================================================
// Arguments
// =====================================================================

#[test]
fn both_argument_declaration_forms() {
    assert_eq!(
        count("Foo { bar { arg a, b = 1; ^a } }", SyntaxKind::ArgDecls),
        1
    );
    assert_eq!(count("f = { |a, b = 1| a };", SyntaxKind::ArgDecls), 1);
}

#[test]
fn pipe_defaults_do_not_swallow_the_closing_pipe() {
    // The bug that took longest to find in the tree-sitter grammar: `|` is
    // both a delimiter and a binary operator.
    let src = "f = { |a, b = 0.0, c = 1| a + b };";
    let parse = parse(src);
    assert!(parse.is_ok(), "errors: {:?}", parse.errors);
    assert_eq!(count(src, SyntaxKind::ArgDecls), 1);
    assert_eq!(count(src, SyntaxKind::VarDef), 3);
}

#[test]
fn rest_arguments() {
    assert_eq!(
        count("Foo { bar { arg a ...rest; ^a } }", SyntaxKind::RestArg),
        1
    );
}

// =====================================================================
// Expressions
// =====================================================================

#[test]
fn operators_are_left_associative_with_no_precedence() {
    // SuperCollider has no operator precedence (lang11d:10), so `1 + 2 * 3`
    // must nest as `(1 + 2) * 3` and evaluate to 9, not 7.
    let t = tree("x = 1 + 2 * 3;");
    let outer = t.find("BinaryExpr").expect("no BinaryExpr");
    let inner = t[outer + 1..]
        .find("BinaryExpr")
        .expect("expected a nested BinaryExpr");
    // The nested one must be indented further, i.e. be a child, and it must be
    // the *left* child: it appears before the outer operator's right operand.
    let _ = inner;
    assert_eq!(count("x = 1 + 2 * 3;", SyntaxKind::BinaryExpr), 2, "{t}");
    // Left-nesting means the first literal under the outer expression is
    // itself inside a BinaryExpr.
    let parse = parse("x = 1 + 2 * 3;");
    let outer_node = parse
        .root
        .descendants()
        .into_iter()
        .find(|n| n.kind == SyntaxKind::BinaryExpr)
        .unwrap();
    let first_child = outer_node.child_nodes().next().unwrap();
    assert_eq!(
        first_child.kind,
        SyntaxKind::BinaryExpr,
        "expected left nesting, got {t}"
    );
}

#[test]
fn adverbs_on_binary_operators() {
    // lang11d:1134 — `expr binop2 adverb expr`
    assert_eq!(count("x = a +.x b;", SyntaxKind::Adverb), 1);
    assert_eq!(count("x = a +.(1) b;", SyntaxKind::Adverb), 1);
}

#[test]
fn method_calls_chain() {
    let src = "x = a.foo.bar(1).baz;";
    assert_eq!(count(src, SyntaxKind::MethodCall), 3);
}

#[test]
fn trailing_block_arguments() {
    assert_eq!(count("x = fork { 1 };", SyntaxKind::CallExpr), 1);
    assert_eq!(count("x = if (a) { 1 } { 2 };", SyntaxKind::CallExpr), 1);
}

#[test]
fn keyword_arguments() {
    assert_eq!(
        count(
            "x = SinOsc.ar(freq: 440, mul: 0.1);",
            SyntaxKind::KeywordArg
        ),
        2
    );
}

#[test]
fn indexing_and_collections() {
    assert_eq!(count("x = a[1];", SyntaxKind::IndexExpr), 1);
    assert_eq!(count("x = [1, 2, 3];", SyntaxKind::Collection), 1);
    assert_eq!(count("x = #[1, 2];", SyntaxKind::LiteralList), 1);
    // Nested index: `a[[0, 2]]`
    assert_eq!(count("x = a[[0, 2]];", SyntaxKind::IndexExpr), 1);
}

#[test]
fn an_array_element_may_end_with_a_semicolon() {
    // `arrayelems1 : exprseq` and `exprseq : exprn optsemi`. The `;` before a
    // `]` is not a typo: `ScIDE.sc` in the stock class library ends an array
    // element with one, and six of its lines did not parse because of this.
    // Verified against sclang 3.13 — `"[1, 2;]".compile` returns a Function.
    for src in [
        "x = [1, 2;];",
        "x = [a;];",
        "x = [\\k: 1;];",
        "x = [1, 2;, 3];",
        // The continuation form already worked, and still has to.
        "x = [a; b];",
    ] {
        assert!(parse(src).is_ok(), "{src:?}: {:?}", parse(src).errors);
    }
    // One element, not two: the `;` ends the sequence rather than separating.
    assert_eq!(count("x = [1, 2;];", SyntaxKind::Collection), 1);
}

#[test]
fn arithmetic_series_in_all_forms() {
    for src in [
        "x = (1..9);",
        "x = (1, 3 .. 9);",
        "x = (..9);",
        "x = (a, b .. c.size);",
    ] {
        assert_eq!(count(src, SyntaxKind::ArithSeries), 1, "failed for {src}");
    }
}

#[test]
fn event_literals() {
    let src = "x = (freq: 440, dur: 1);";
    assert_eq!(count(src, SyntaxKind::EventLiteral), 1);
    assert_eq!(count(src, SyntaxKind::KeywordArg), 2);
}

#[test]
fn environment_variables() {
    assert_eq!(count("~foo = 1;", SyntaxKind::EnvVarRef), 1);
}

#[test]
fn destructuring_assignment() {
    let src = "#a, b = [1, 2];";
    assert_eq!(count(src, SyntaxKind::MultiAssignExpr), 1);
    assert_eq!(count(src, SyntaxKind::MultiAssignTargets), 1);
}

#[test]
fn closed_functions() {
    assert_eq!(count("x = #{ |a| a };", SyntaxKind::FunctionBlock), 1);
}

// =====================================================================
// Error recovery — the property sclang's own parser cannot provide.
// =====================================================================

#[test]
fn a_broken_method_does_not_destroy_its_siblings() {
    // The middle method is malformed. The other two must still be found,
    // because that is what keeps completion working while you type.
    let src = "Foo { a { ^1 } b { ^ } c { ^3 } }";
    let parse = parse(src);
    assert!(!parse.is_ok(), "expected an error");
    assert_eq!(
        parse
            .root
            .descendants()
            .iter()
            .filter(|n| n.kind == SyntaxKind::MethodDef)
            .count(),
        3,
        "{}",
        parse.root.debug_tree(src)
    );
}

#[test]
fn an_unterminated_class_still_yields_its_methods() {
    let src = "Foo { bar { ^1 } baz { ^2 }";
    let parse = parse(src);
    assert_eq!(
        parse
            .root
            .descendants()
            .iter()
            .filter(|n| n.kind == SyntaxKind::MethodDef)
            .count(),
        2
    );
}

#[test]
fn errors_carry_usable_ranges() {
    let src = "Foo { bar { ^ } }";
    let parse = parse(src);
    assert!(!parse.errors.is_empty());
    for e in &parse.errors {
        assert!(
            e.end as usize <= src.len(),
            "error range out of bounds: {e:?}"
        );
        assert!(e.start <= e.end, "inverted error range: {e:?}");
    }
}

#[test]
fn a_stray_closer_in_a_class_body_is_reported_once() {
    // Neither `)` nor `]` is a member, a recovery target, or something the
    // depth tracking will step over — so recovery moved nothing and the body
    // loop asked again from the same place, reporting the same error 257 times
    // before the parser's fuel ran out. One squiggle, one entry in the
    // problems panel.
    for src in ["Foo { ) }", "Foo { bar { ^1 } ] }"] {
        let parse = parse(src);
        assert_eq!(parse.errors.len(), 1, "{src:?}: {:?}", parse.errors);
    }
}

#[test]
fn a_typo_mid_file_leaves_later_classes_intact() {
    let src = "Foo { ??? } \n Bar { baz { ^1 } }";
    let parse = parse(src);
    let classes: Vec<_> = parse
        .root
        .descendants()
        .into_iter()
        .filter(|n| n.kind == SyntaxKind::ClassDef)
        .collect();
    assert_eq!(classes.len(), 2, "{}", parse.root.debug_tree(src));
}

// =====================================================================
// Constructs found by running against the real class library. Each of
// these was a conformance failure that `lang11d` explained.
// =====================================================================

#[test]
fn array_expansion_in_arguments() {
    // `arglistv1 : '*' exprseq`
    assert_eq!(
        count("x = this.new1('control', *args);", SyntaxKind::SplatArg),
        1
    );
    assert_eq!(count("x = Sum4(*a);", SyntaxKind::SplatArg), 1);
}

#[test]
fn index_subranges() {
    // `valrangex1 : expr1 '[' arglist1 DOTDOT ']' | ...`
    for src in ["x = a[0..1];", "x = a[skip..];", "x = a[..n-1];"] {
        assert_eq!(count(src, SyntaxKind::IndexRange), 1, "failed for {src}");
    }
    // Without a `..` it is an ordinary index, not a range.
    assert_eq!(count("x = a[0];", SyntaxKind::IndexRange), 0);
}

#[test]
fn value_call_shorthand() {
    // `f.(args)` means `f.value(args)`.
    assert_eq!(count("x = suggestNew.(n, in);", SyntaxKind::MethodCall), 1);
}

#[test]
fn space_separated_pipe_arguments() {
    // `slotdeflist : slotdef | slotdeflist optcomma slotdef` — the comma is
    // optional, so `{|a b|}` declares two arguments.
    let src = "f = {|a b| a < b};";
    let parse = parse(src);
    assert!(parse.is_ok(), "errors: {:?}", parse.errors);
    assert_eq!(count(src, SyntaxKind::VarDef), 2);
}

#[test]
fn defaults_without_an_equals_sign() {
    // `slotdef : name optequal slotliteral` — `optequal` is genuinely optional.
    let src = "f = {|action, start = 0, range -1| action};";
    let parse = parse(src);
    assert!(parse.is_ok(), "errors: {:?}", parse.errors);
    assert_eq!(count(src, SyntaxKind::VarDef), 3);
}

#[test]
fn parenthesised_defaults() {
    let src = "Foo { writeDefFile { arg dir, overwrite(true); ^dir } }";
    let parse = parse(src);
    assert!(parse.is_ok(), "errors: {:?}", parse.errors);
    assert_eq!(count(src, SyntaxKind::VarDef), 2);
}

#[test]
fn negative_defaults_in_pipe_arguments() {
    let src = "f = {|min = -90, max = 6| min};";
    let parse = parse(src);
    assert!(parse.is_ok(), "errors: {:?}", parse.errors);
}

#[test]
fn while_is_an_ordinary_identifier() {
    // `name : NAME | WHILE` — outside the generator syntax `while` is a method.
    for src in ["x = while { a } { b };", "x = while({ a },{ b });"] {
        let parse = parse(src);
        assert!(parse.is_ok(), "errors for {src}: {:?}", parse.errors);
    }
}

#[test]
fn pi_suffixed_numbers() {
    // `floatp : floatr pie | integer pie | pie`
    for src in ["x = 0.5pi;", "x = 2pi;", "x = pi;"] {
        let parse = parse(src);
        assert!(parse.is_ok(), "errors for {src}: {:?}", parse.errors);
        assert_eq!(count(src, SyntaxKind::Literal), 1, "failed for {src}");
    }
}

#[test]
fn event_keys_may_be_any_expression() {
    // `dictslotdef : exprseq ':' exprseq | keybinop exprseq`
    assert_eq!(count("x = (0: 0, 1: 1, 2: 2);", SyntaxKind::KeywordArg), 3);
    assert_eq!(
        count(
            "x = (\"serverInfo\": a, \"capabilities\": b);",
            SyntaxKind::KeywordArg
        ),
        2
    );
}

#[test]
fn event_values_may_carry_a_trailing_semicolon() {
    // `exprseq : exprn optsemi`
    let src = "Foo { bar { ^(\"a\": x, \"b\": y;) } }";
    let parse = parse(src);
    assert!(parse.is_ok(), "errors: {:?}", parse.errors);
}

#[test]
fn destructuring_with_a_rest_element() {
    let src = "#cmdName ...path = cmdPath;";
    let parse = parse(src);
    assert!(parse.is_ok(), "errors: {:?}", parse.errors);
    assert_eq!(count(src, SyntaxKind::MultiAssignExpr), 1);
}

#[test]
fn adjacent_string_literals_concatenate() {
    // Done by the lexer, not the grammar (PyrLexer.cpp:844) — `lang11d` has
    // only `string : STRING`.
    let src = "x = Error(\"first part \"\n    \"second part\").throw;";
    let parse = parse(src);
    assert!(parse.is_ok(), "errors: {:?}", parse.errors);
    // The lexer emits one token per segment (matching sc_lexer's StringLine);
    // the parser joins them into a single Literal node.
    assert_eq!(count(src, SyntaxKind::Literal), 1);
}

// =====================================================================
// What may be called
// =====================================================================

/// A `.scd` file is normally a sequence of top-level `( … )` blocks, evaluated
/// one at a time. They are not separated by semicolons and the file is never
/// parsed as a unit, so each block has to stand on its own here.
///
/// This used to produce a single `CallExpr` spanning both: the parser allowed
/// any expression to be followed by an argument list, so `)` then `(` read as a
/// call. `lang11d` has no `expr '(' arglist ')'` production — a callee is a
/// `name` or a `classname` — and sclang rejects the same input with
/// "unexpected '(', expecting end of file".
#[test]
fn adjacent_top_level_blocks_are_separate() {
    let src = "(\n1\n)\n\n(\n2\n)\n";
    assert_eq!(count(src, SyntaxKind::ParenExpr), 2);
    assert_eq!(count(src, SyntaxKind::CallExpr), 0);
    assert_lossless(src);
}

#[test]
fn a_parenthesised_expression_is_not_callable() {
    // Neither with a newline between them nor without.
    assert_eq!(count("(1)\n(2)", SyntaxKind::CallExpr), 0);
    assert_eq!(count("(1)(2)", SyntaxKind::CallExpr), 0);
}

#[test]
fn names_and_class_names_are_still_callable() {
    assert_eq!(count("f(2)", SyntaxKind::CallExpr), 1);
    assert_eq!(count("x = Point(1, 2);", SyntaxKind::CallExpr), 1);
    // A trailing block, and a call that already has one: the grammar hangs the
    // `blocklist` off the whole production, so both blocks belong to the call.
    assert_eq!(count("x = if (a) { 1 } { 2 };", SyntaxKind::CallExpr), 1);
    assert_eq!(count("x = Routine { 1 };", SyntaxKind::CallExpr), 1);
    assert_eq!(
        count("x = SynthDef(\\a, { 1 }).add;", SyntaxKind::CallExpr),
        1
    );
}

#[test]
fn two_blocks_of_real_code_keep_their_own_boundaries() {
    // The shape that reported this: a pattern, then a SynthDef, with a blank
    // line between. Both have to be their own top-level node or an editor
    // cannot evaluate one without the other.
    let src = "(\nPbind(\\degree, 1).play;\n)\n\n(\nSynthDef(\\a, { 1 }).add;\n)\n";
    let root = parse(src);
    let top: Vec<SyntaxKind> = root
        .root
        .children
        .iter()
        .filter_map(|c| match c {
            Child::Node(n) => Some(n.kind),
            Child::Token(_) => None,
        })
        .collect();

    assert_eq!(
        top,
        vec![SyntaxKind::ParenExpr, SyntaxKind::ParenExpr],
        "each block must be its own top-level node"
    );
    assert_lossless(src);
}

/// `Set[1, 2, 3]` begins exactly like the indexed class definition
/// `Array[slot] : ArrayedCollection { … }`, and was parsed as one. Every
/// literal collection at the start of a statement was affected, which is 82 of
/// the help-file examples the script oracle checks.
#[test]
fn a_literal_collection_is_not_a_class_definition() {
    for src in [
        "Set[1, 2, 3].powerset;",
        "Bag[\"a\", \"b\"].contents;",
        "List[1, 2, 3].sum;",
        "Set[\\a, \\b];",
    ] {
        assert_eq!(
            count(src, SyntaxKind::ClassDef),
            0,
            "{src} is an expression, not a class definition"
        );
        assert_lossless(src);
    }
}

/// The form it was being confused with still parses, in both its shapes.
#[test]
fn indexed_class_definitions_still_parse() {
    let named = "Array[slot] : ArrayedCollection { foo { ^1 } }";
    assert_eq!(count(named, SyntaxKind::ClassDef), 1);
    assert_eq!(count(named, SyntaxKind::IndexedSlot), 1);

    // `optname` is optional, and the superclass may be left out.
    assert_eq!(count("Foo[] { bar { ^1 } }", SyntaxKind::ClassDef), 1);
    assert_eq!(count("Foo[slot] { bar { ^1 } }", SyntaxKind::ClassDef), 1);
}

/// `adverb : '.' name | '.' integer | '.' '(' exprseq ')'`, and `integer` is
/// `INTEGER | '-' INTEGER` — so an adverb may be negative. `z +.-1 y` shifts
/// the other way from `z +.1 y`.
#[test]
fn adverbs_take_a_negative_integer() {
    for src in ["z +.-1 y", "z +.1 y", "z +.x y", "z +.(a + 1) y"] {
        assert_eq!(count(src, SyntaxKind::Adverb), 1, "{src}");
        assert!(parse(src).is_ok(), "{src} should parse");
        assert_lossless(src);
    }

    // A `.` that is not an adverb is still a method call.
    assert_eq!(count("z + y.abs", SyntaxKind::Adverb), 0);
}
