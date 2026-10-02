//! Trees for constructs the installed help files exercise only lightly, or
//! not at all. Every expected dump was produced by upstream's own parser —
//! `oracle/build-scdoc.sh` at Version-3.14.1 — not written by hand, so these
//! pin upstream's behaviour rather than a reading of it.

use sclang_scdoc::{parse, Mode};

fn dump(src: &str, mode: Mode) -> Option<String> {
    parse(src, mode).ok().map(|n| n.dump())
}

/// Header, description, a class method with arguments, returns and
/// discussion, and an example: the shape most help files have.
#[test]
fn a_class_page() {
    let src = "class:: Foo\nsummary:: Does a foo.\nrelated:: Classes/Bar, Classes/Baz\ncategories:: Testing>Foo\n\ndescription::\nA foo is like a link::Classes/Bar::\nbut with code::Foo.new::.\n\nA second paragraph.\n\nclassmethods::\nmethod:: ar, kr\nMakes one.\nargument:: freq\nIn Hertz.\nargument:: phase_\nIn radians.\nreturns:: A Foo.\ndiscussion:: Nothing to add.\n\nexamples::\ncode::\n(\n{ Foo.ar(440) }.play;\n)\n::\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "Foo"
|   |-- SUMMARY "Does a foo."
|   |-- RELATED
|   |   |-- STRING "Classes/Bar"
|   |   `-- STRING "Classes/Baz"
|   `-- CATEGORIES
|       `-- STRING "Testing>Foo"
`-- BODY
    |-- DESCRIPTION
    |   |-- PROSE
    |   |   |-- TEXT "A foo is like a "
    |   |   |-- LINK "Classes/Bar"
    |   |   |-- NL
    |   |   |-- TEXT "but with "
    |   |   |-- CODE "Foo.new"
    |   |   `-- TEXT "."
    |   `-- PROSE
    |       `-- TEXT "A second paragraph."
    |-- CLASSMETHODS
    |   `-- CMETHOD
    |       |-- METHODNAMES
    |       |   |-- STRING "ar"
    |       |   `-- STRING "kr"
    |       `-- METHODBODY
    |           |-- PROSE
    |           |   `-- TEXT "Makes one."
    |           |-- ARGUMENTS
    |           |   |-- ARGUMENT "freq"
    |           |   |   `-- PROSE
    |           |   |       `-- TEXT "In Hertz."
    |           |   `-- ARGUMENT "phase_"
    |           |       `-- PROSE
    |           |           `-- TEXT "In radians."
    |           |-- RETURNS
    |           |   `-- PROSE
    |           |       `-- TEXT "A Foo."
    |           `-- DISCUSSION
    |               `-- PROSE
    |                   `-- TEXT "Nothing to add."
    `-- EXAMPLES
        |-- CODEBLOCK "(
{ Foo.ar(440) }.play;
)"
        `-- PROSE
"#;
    assert_eq!(dump(src, Mode::Full).as_deref(), Some(&expected[1..]));
}

#[test]
fn method_argument_strings_only_outside_method_sections() {
    let src = "title:: Guide\nsummary:: x\n\nsection:: Functions\nmethod:: foo (a, b)\nDoes foo.\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "Guide"
|   `-- SUMMARY "x"
`-- BODY
    `-- SECTION "Functions"
        `-- METHOD "(a, b)"
            |-- METHODNAMES
            |   `-- STRING "foo"
            `-- METHODBODY
                `-- PROSE
                    `-- TEXT "Does foo."
"#;
    assert_eq!(dump(src, Mode::Full).as_deref(), Some(&expected[1..]));
}

#[test]
fn an_argument_string_in_classmethods_is_rejected() {
    let src = "class:: Foo\nsummary:: x\nclassmethods::\nmethod:: foo (a)\n";
    assert_eq!(dump(src, Mode::Full), None);
}

#[test]
fn copymethod_needs_two_words() {
    let src = "class:: Foo\nsummary:: x\ninstancemethods::\ncopymethod:: Bar\n";
    assert_eq!(dump(src, Mode::Full), None);
}

#[test]
fn copymethod_and_private_follow_the_section() {
    let src = "class:: Foo\nsummary:: x\nclassmethods::\ncopymethod:: Bar *new\nprivate:: a, b\ninstancemethods::\ncopymethod:: Bar -play\nprivate:: c\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "Foo"
|   `-- SUMMARY "x"
`-- BODY
    |-- CLASSMETHODS
    |   |-- CCOPYMETHOD "Bar *new"
    |   `-- CPRIVATE
    |       |-- STRING "a"
    |       `-- STRING "b"
    `-- INSTANCEMETHODS
        |-- ICOPYMETHOD "Bar -play"
        `-- IPRIVATE
            `-- STRING "c"
"#;
    assert_eq!(dump(src, Mode::Full).as_deref(), Some(&expected[1..]));
}

#[test]
fn lists_tables_and_definition_lists() {
    let src = "title:: T\nsummary:: s\ndescription::\nlist::\n## one\n## two ## three\n::\ntable::\n## a || b\n## || c\n::\ndefinitionlist::\n## term ## other || its definition\n## x || y\n::\nnumberedlist::\n## n\n::\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "T"
|   `-- SUMMARY "s"
`-- BODY
    `-- DESCRIPTION
        |-- LIST
        |   |-- ITEM
        |   |   `-- PROSE
        |   |       `-- TEXT "one"
        |   |-- ITEM
        |   |   `-- PROSE
        |   |       `-- TEXT "two"
        |   `-- ITEM
        |       `-- PROSE
        |           `-- TEXT "three"
        |-- PROSE
        |-- TABLE
        |   |-- TABROW
        |   |   |-- TABCOL
        |   |   |   `-- PROSE
        |   |   |       `-- TEXT "a"
        |   |   `-- TABCOL
        |   |       `-- PROSE
        |   |           `-- TEXT "b"
        |   `-- TABROW
        |       |-- TABCOL
        |       `-- TABCOL
        |           `-- PROSE
        |               `-- TEXT "c"
        |-- PROSE
        |-- DEFINITIONLIST
        |   |-- DEFLISTITEM
        |   |   |-- TERM
        |   |   |   `-- PROSE
        |   |   |       `-- TEXT "term"
        |   |   |-- TERM
        |   |   |   `-- PROSE
        |   |   |       `-- TEXT "other"
        |   |   `-- DEFINITION
        |   |       `-- PROSE
        |   |           `-- TEXT "its definition"
        |   `-- DEFLISTITEM
        |       |-- TERM
        |       |   `-- PROSE
        |       |       `-- TEXT "x"
        |       `-- DEFINITION
        |           `-- PROSE
        |               `-- TEXT "y"
        |-- PROSE
        |-- NUMBEREDLIST
        |   `-- ITEM
        |       `-- PROSE
        |           `-- TEXT "n"
        `-- PROSE
"#;
    assert_eq!(dump(src, Mode::Full).as_deref(), Some(&expected[1..]));
}

#[test]
fn notes_warnings_and_footnotes_nest() {
    let src = "title:: T\nsummary:: s\ndescription::\nnote:: Careful footnote::Really.:: now.\nwarning:: Nested list::\n## a\n::\n::\n::\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "T"
|   `-- SUMMARY "s"
`-- BODY
    `-- DESCRIPTION
        |-- NOTE
        |   |-- PROSE
        |   |   |-- TEXT "Careful"
        |   |   |-- FOOTNOTE
        |   |   |   `-- PROSE
        |   |   |       `-- TEXT "Really."
        |   |   `-- TEXT " now."
        |   `-- WARNING
        |       |-- PROSE
        |       |   `-- TEXT "Nested"
        |       `-- LIST
        |           `-- ITEM
        |               `-- PROSE
        |                   `-- TEXT "a"
        `-- PROSE
"#;
    assert_eq!(dump(src, Mode::Full).as_deref(), Some(&expected[1..]));
}

#[test]
fn escapes_urls_and_tabs() {
    let src = "title:: T\nsummary:: s\ndescription::\nA \\:: and \\|| and \\## and http://example.com/a:b x.\ncode::\n\tf.(1);\n\\::\n::\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "T"
|   `-- SUMMARY "s"
`-- BODY
    `-- DESCRIPTION
        |-- PROSE
        |   |-- TEXT "A :: and || and ## and "
        |   |-- LINK "http://example.com/a"
        |   `-- TEXT ":b x."
        |-- CODEBLOCK "    f.(1);
::"
        `-- PROSE
"#;
    assert_eq!(dump(src, Mode::Full).as_deref(), Some(&expected[1..]));
}

#[test]
fn a_code_block_keeps_its_lines() {
    let src = "title:: T\nsummary:: s\nexamples::\ncode::\n\n  a = 1;\n\n  b::c;\n\n::\nteletype::\nplain\n::\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "T"
|   `-- SUMMARY "s"
`-- BODY
    `-- EXAMPLES
        |-- CODEBLOCK "  a = 1;

  b::c;"
        |-- PROSE
        |-- TELETYPEBLOCK "plain"
        `-- PROSE
"#;
    assert_eq!(dump(src, Mode::Full).as_deref(), Some(&expected[1..]));
}

#[test]
fn subsubsection_and_math_from_3_14() {
    let src = "title:: T\nsummary:: s\nsection:: S\nsubsection:: Sub\nsubsubsection:: Deeper\nSome math::x^2:: here.\nmath::\nE = mc^2\n::\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "T"
|   `-- SUMMARY "s"
`-- BODY
    `-- SECTION "S"
        `-- SUBSECTION "Sub"
            `-- SUBSUBSECTION "Deeper"
                |-- PROSE
                |   |-- TEXT "Some "
                |   |-- MATH "x^2"
                |   `-- TEXT " here."
                |-- MATHBLOCK "E = mc^2"
                `-- PROSE
"#;
    assert_eq!(dump(src, Mode::Full).as_deref(), Some(&expected[1..]));
}

#[test]
fn tags_are_case_insensitive() {
    let src = "Class:: Foo\nSummary:: x\nDESCRIPTION::\nStrong::loud:: words.\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "Foo"
|   `-- SUMMARY "x"
`-- BODY
    `-- DESCRIPTION
        `-- PROSE
            |-- STRONG "loud"
            `-- TEXT " words."
"#;
    assert_eq!(dump(src, Mode::Full).as_deref(), Some(&expected[1..]));
}

#[test]
fn a_header_tag_needs_text() {
    let src = "class::\nsummary:: x\n";
    assert_eq!(dump(src, Mode::Full), None);
}

#[test]
fn a_partial_file_has_no_header() {
    let src = "instancemethods::\nmethod:: foo\nA method from a quark.\n";
    let expected = r#"
BODY
`-- INSTANCEMETHODS
    `-- IMETHOD
        |-- METHODNAMES
        |   `-- STRING "foo"
        `-- METHODBODY
            `-- PROSE
                `-- TEXT "A method from a quark."
"#;
    assert_eq!(dump(src, Mode::Partial).as_deref(), Some(&expected[1..]));
}

#[test]
fn a_partial_parse_rejects_a_header() {
    let src = "class:: Foo\nsummary:: x\n";
    assert_eq!(dump(src, Mode::Partial), None);
}

#[test]
fn metadata_skips_prose() {
    let src = "class:: Foo\nsummary:: A foo.\ndescription::\nLots of link::Classes/Bar:: prose.\ncode::\nx::\n::\nclassmethods::\nmethod:: new, make\nargument:: x\nIgnored.\ninstancemethods::\nprivate:: secret\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "Foo"
|   `-- SUMMARY "A foo."
`-- BODY
    |-- DESCRIPTION
    |-- CLASSMETHODS
    |   `-- CMETHOD
    |       |-- METHODNAMES
    |       |   |-- STRING "new"
    |       |   `-- STRING "make"
    |       `-- METHODBODY
    `-- INSTANCEMETHODS
        `-- IPRIVATE
            `-- STRING "secret"
"#;
    assert_eq!(dump(src, Mode::Metadata).as_deref(), Some(&expected[1..]));
}

#[test]
fn carriage_returns_are_ignored() {
    let src = "class:: Foo\r\nsummary:: x\r\ndescription::\r\nline one\r\nline two\r\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "Foo"
|   `-- SUMMARY "x"
`-- BODY
    `-- DESCRIPTION
        `-- PROSE
            `-- TEXT "line one line two"
"#;
    assert_eq!(dump(src, Mode::Full).as_deref(), Some(&expected[1..]));
}

#[test]
fn an_empty_full_document_body() {
    let src = "class:: Foo\nsummary:: x\n";
    let expected = r#"
DOCUMENT
|-- HEADER
|   |-- TITLE "Foo"
|   `-- SUMMARY "x"
`-- BODY
"#;
    assert_eq!(dump(src, Mode::Full).as_deref(), Some(&expected[1..]));
}
