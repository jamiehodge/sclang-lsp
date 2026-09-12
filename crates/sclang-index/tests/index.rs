//! Index behaviour.

use sclang_index::{MethodKind, Origin, SymbolIndex, VarKind};
use std::path::Path;

fn index(files: &[(&str, &str)]) -> SymbolIndex {
    let mut ix = SymbolIndex::default();
    for (name, src) in files {
        ix.index_file(Path::new(name), src);
    }
    ix
}

fn one(src: &str) -> SymbolIndex {
    index(&[("test.sc", src)])
}

// =====================================================================
// Classes
// =====================================================================

#[test]
fn classes_and_superclasses() {
    let ix = one("Foo : Bar { } Baz { }");
    assert_eq!(ix.class("Foo").unwrap().superclass.as_deref(), Some("Bar"));
    // A class with no `:` has an implicit superclass, which the source does
    // not state and the index does not invent.
    assert_eq!(ix.class("Baz").unwrap().superclass, None);
}

#[test]
fn indexed_class_slot() {
    let ix = one("Array[slot] : ArrayedCollection { }");
    assert_eq!(
        ix.class("Array").unwrap().indexed_slot.as_deref(),
        Some("slot")
    );
}

#[test]
fn goto_definition_range_covers_the_name_only() {
    let src = "Foo : Bar { }";
    let ix = one(src);
    let loc = &ix.class("Foo").unwrap().location;
    assert_eq!(
        &src[loc.name_range.start as usize..loc.name_range.end as usize],
        "Foo"
    );
    // The full range covers the whole declaration.
    assert_eq!(loc.range.start, 0);
    assert_eq!(loc.range.end as usize, src.len());
}

#[test]
fn superclass_chain_walks_upward() {
    let ix = one("A { } B : A { } C : B { }");
    let chain: Vec<_> = ix
        .superclass_chain("C")
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(chain, ["C", "B", "A"]);
}

#[test]
fn a_superclass_cycle_does_not_hang() {
    // Not valid SuperCollider, but trivially typed into an editor.
    let ix = one("A : B { } B : A { }");
    let chain = ix.superclass_chain("A");
    assert!(
        chain.len() <= 2,
        "cycle should terminate, got {}",
        chain.len()
    );
}

#[test]
fn subclasses_are_tracked() {
    let ix = one("A { } B : A { } C : A { }");
    let mut subs: Vec<_> = ix.subclasses("A").iter().map(|c| c.name.as_str()).collect();
    subs.sort();
    assert_eq!(subs, ["B", "C"]);
}

// =====================================================================
// Methods
// =====================================================================

#[test]
fn instance_and_class_methods_are_distinguished() {
    let ix = one("Foo { bar { ^1 } *baz { ^2 } }");
    assert!(ix.method("Foo", "bar", MethodKind::Instance).is_some());
    assert!(ix.method("Foo", "baz", MethodKind::Class).is_some());
    // Same name, different side, is a different method.
    assert!(ix.method("Foo", "bar", MethodKind::Class).is_none());
}

#[test]
fn operator_named_methods_are_indexed() {
    let ix = one("Foo { ++ { arg x; ^1 } < { ^2 } }");
    assert!(ix.method("Foo", "++", MethodKind::Instance).is_some());
    assert!(ix.method("Foo", "<", MethodKind::Instance).is_some());
}

#[test]
fn arguments_and_defaults_are_captured() {
    let ix = one("Foo { *new { |server, startBus = 0, min = -90, opts = (2)| ^1 } }");
    let m = ix.method("Foo", "new", MethodKind::Class).unwrap();
    let names: Vec<_> = m.args.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["server", "startBus", "min", "opts"]);
    assert_eq!(m.args[0].default, None);
    assert_eq!(m.args[1].default.as_deref(), Some("0"));
    assert_eq!(m.args[2].default.as_deref(), Some("-90"));
    assert_eq!(m.args[3].default.as_deref(), Some("(2)"));
}

#[test]
fn rest_arguments_are_marked() {
    let ix = one("Foo { bar { arg a ...rest; ^a } }");
    let m = ix.method("Foo", "bar", MethodKind::Instance).unwrap();
    assert!(m.args.iter().any(|a| a.is_rest && a.name == "rest"));
}

#[test]
fn signatures_read_like_the_source() {
    let ix = one("Foo { *new { |a, b = 2| ^1 } bar { arg x ...ys; ^x } }");
    assert_eq!(
        ix.method("Foo", "new", MethodKind::Class)
            .unwrap()
            .signature(),
        "*new(a, b = 2)"
    );
    assert_eq!(
        ix.method("Foo", "bar", MethodKind::Instance)
            .unwrap()
            .signature(),
        "bar(x, ...ys)"
    );
}

#[test]
fn nested_closures_do_not_contribute_arguments() {
    // A method with no arguments of its own, containing a block that has some.
    let ix = one("Foo { bar { ^this.things.collect { |item| item } } }");
    let m = ix.method("Foo", "bar", MethodKind::Instance).unwrap();
    assert!(m.args.is_empty(), "got {:?}", m.args);
}

// =====================================================================
// Variables and generated accessors
// =====================================================================

#[test]
fn accessor_methods_are_synthesised_from_markers() {
    let ix = one("Foo { var <a, >b, <>c, d; }");
    // `<` gives a getter, `>` a setter, `<>` both, bare gives neither.
    assert!(ix.method("Foo", "a", MethodKind::Instance).is_some());
    assert!(ix.method("Foo", "a_", MethodKind::Instance).is_none());
    assert!(ix.method("Foo", "b_", MethodKind::Instance).is_some());
    assert!(ix.method("Foo", "b", MethodKind::Instance).is_none());
    assert!(ix.method("Foo", "c", MethodKind::Instance).is_some());
    assert!(ix.method("Foo", "c_", MethodKind::Instance).is_some());
    assert!(ix.method("Foo", "d", MethodKind::Instance).is_none());
}

#[test]
fn classvar_and_const_accessors_are_class_side() {
    // `const <comma = $,` in Char.sc is a class method, not an instance one.
    let ix = one("Foo { classvar <all; const <comma = $,; var <inst; }");
    assert!(ix.method("Foo", "all", MethodKind::Class).is_some());
    assert!(ix.method("Foo", "comma", MethodKind::Class).is_some());
    assert!(ix.method("Foo", "inst", MethodKind::Instance).is_some());
}

#[test]
fn accessors_are_marked_as_generated() {
    let ix = one("Foo { var <a; b { ^1 } }");
    assert_eq!(
        ix.method("Foo", "a", MethodKind::Instance).unwrap().origin,
        Origin::Accessor
    );
    assert_eq!(
        ix.method("Foo", "b", MethodKind::Instance).unwrap().origin,
        Origin::Declared
    );
}

#[test]
fn variables_record_their_kind_and_default() {
    let ix = one("Foo { classvar <count = 0; var <>name; const pi2 = 6.28; }");
    let vars = &ix.class("Foo").unwrap().vars;
    let count = vars.iter().find(|v| v.name == "count").unwrap();
    assert_eq!(count.kind, VarKind::Class);
    assert_eq!(count.default.as_deref(), Some("0"));
    assert_eq!(
        vars.iter().find(|v| v.name == "name").unwrap().kind,
        VarKind::Instance
    );
    assert_eq!(
        vars.iter().find(|v| v.name == "pi2").unwrap().kind,
        VarKind::Const
    );
}

// =====================================================================
// Extensions
// =====================================================================

#[test]
fn class_extensions_add_to_the_class() {
    let ix = index(&[
        ("Object.sc", "Object { foo { ^1 } }"),
        ("ext.sc", "+ Object { bar { ^2 } }"),
    ]);
    assert!(ix.method("Object", "foo", MethodKind::Instance).is_some());
    assert!(ix.method("Object", "bar", MethodKind::Instance).is_some());
}

#[test]
fn an_extension_of_an_unknown_class_still_indexes_its_methods() {
    // Extensions are routinely indexed before the class they extend.
    let ix = one("+ NotYetSeen { foo { ^1 } }");
    assert!(ix.class("NotYetSeen").is_none());
    assert!(ix
        .method("NotYetSeen", "foo", MethodKind::Instance)
        .is_some());
}

// =====================================================================
// Queries an LSP needs
// =====================================================================

#[test]
fn inherited_methods_are_visible_with_overrides_shadowing() {
    let ix = one("A { foo { ^1 } bar { ^1 } } B : A { foo { ^2 } }");
    let visible = ix.methods_visible_on("B", MethodKind::Instance);
    let names: Vec<_> = visible.iter().map(|m| m.name.as_str()).collect();
    assert!(names.contains(&"foo"));
    assert!(names.contains(&"bar"));
    // `foo` appears once, and it is B's.
    assert_eq!(names.iter().filter(|n| **n == "foo").count(), 1);
    assert_eq!(visible.iter().find(|m| m.name == "foo").unwrap().owner, "B");
}

#[test]
fn implementors_answers_who_defines_a_selector() {
    let ix = one("A { play { ^1 } } B { play { ^2 } } C { stop { ^3 } }");
    let mut owners: Vec<_> = ix
        .implementors("play")
        .iter()
        .map(|m| m.owner.clone())
        .collect();
    owners.sort();
    assert_eq!(owners, ["A", "B"]);
    assert!(ix.implementors("nonexistent").is_empty());
}

#[test]
fn prefix_queries_drive_completion() {
    let ix = one("SinOsc { } SinOscFB { } Saw { } Foo { playThing { ^1 } playOther { ^2 } }");
    let classes: Vec<_> = ix
        .classes_with_prefix("SinOsc")
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(classes, ["SinOsc", "SinOscFB"]);
    assert_eq!(
        ix.method_names_with_prefix("play"),
        ["playOther", "playThing"]
    );
}

// =====================================================================
// Incremental re-indexing — what an editor does on every keystroke
// =====================================================================

#[test]
fn reindexing_a_file_replaces_its_symbols() {
    let mut ix = SymbolIndex::default();
    ix.index_file(Path::new("a.sc"), "Foo { old { ^1 } }");
    assert!(ix.method("Foo", "old", MethodKind::Instance).is_some());

    ix.index_file(Path::new("a.sc"), "Foo { new_ { ^1 } }");
    assert!(
        ix.method("Foo", "old", MethodKind::Instance).is_none(),
        "stale method survived"
    );
    assert!(ix.method("Foo", "new_", MethodKind::Instance).is_some());
}

#[test]
fn removing_a_file_withdraws_everything_it_declared() {
    let mut ix = index(&[
        ("a.sc", "A { foo { ^1 } }"),
        ("b.sc", "B : A { bar { ^2 } }"),
    ]);
    assert_eq!(ix.class_count(), 2);

    ix.remove_file(Path::new("b.sc"));
    assert_eq!(ix.class_count(), 1);
    assert!(ix.class("B").is_none());
    assert!(ix.method("B", "bar", MethodKind::Instance).is_none());
    assert!(
        ix.subclasses("A").is_empty(),
        "stale subclass edge survived"
    );
    assert!(
        ix.implementors("bar").is_empty(),
        "stale name entry survived"
    );
    // A is untouched.
    assert!(ix.method("A", "foo", MethodKind::Instance).is_some());
}

#[test]
fn removing_an_unindexed_file_is_harmless() {
    let mut ix = one("A { }");
    ix.remove_file(Path::new("never-seen.sc"));
    assert_eq!(ix.class_count(), 1);
}

// =====================================================================
// Broken input
// =====================================================================

#[test]
fn a_broken_method_does_not_cost_the_others() {
    // The parser recovers, so indexing a file mid-edit still yields symbols.
    let ix = one("Foo { a { ^1 } b { ^ } c { ^3 } }");
    assert!(ix.method("Foo", "a", MethodKind::Instance).is_some());
    assert!(ix.method("Foo", "b", MethodKind::Instance).is_some());
    assert!(ix.method("Foo", "c", MethodKind::Instance).is_some());
}

#[test]
fn an_unterminated_class_still_yields_its_methods() {
    let ix = one("Foo { bar { ^1 } baz { ^2 }");
    assert!(ix.method("Foo", "bar", MethodKind::Instance).is_some());
    assert!(ix.method("Foo", "baz", MethodKind::Instance).is_some());
}

// =====================================================================
// Documentation
// =====================================================================

#[test]
fn a_preceding_comment_becomes_the_doc() {
    let ix = one("Foo {\n\t// Does the thing.\n\t// Twice.\n\tbar { ^1 }\n}");
    let doc = ix
        .method("Foo", "bar", MethodKind::Instance)
        .unwrap()
        .doc
        .as_deref();
    assert_eq!(doc, Some("Does the thing.\nTwice."));
}

#[test]
fn a_blank_line_separates_a_comment_from_the_method() {
    let ix = one("Foo {\n\t// Unrelated note.\n\n\tbar { ^1 }\n}");
    assert_eq!(
        ix.method("Foo", "bar", MethodKind::Instance).unwrap().doc,
        None
    );
}
