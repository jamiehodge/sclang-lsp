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
