//! A parse never panics, whatever it is given.
//!
//! Upstream's answer to a broken file is to reject it, and that is reproduced;
//! but a help file being written is broken at almost every keystroke, so
//! rejecting has to be all that happens.

use sclang_scdoc::{parse, Mode};

const SAMPLE: &str = "class:: Foo\nsummary:: A foo.\nrelated:: Classes/Bar\ncategories:: X>Y\n\
description::\nProse with link::Classes/Bar:: and code::x:: and http://a.b/c, footnote::f::.\n\
list::\n## a\n## b ## c\n::\ntable::\n## a || b\n::\ndefinitionlist::\n## t || d\n::\n\
note:: n ::\nwarning:: w ::\nclassmethods::\nmethod:: ar, kr\nargument:: freq\nHz.\n\
returns:: x\ndiscussion:: y\nprivate:: p\ninstancemethods::\ncopymethod:: Bar -x\n\
section:: S (args)\nmethod:: f (a, b)\nsubsection:: T\nsubsubsection:: U\nmath::x^2::\n\
examples::\ncode::\n(\n\t1 + 2\n\\::\n)\n::\nteletype::\nt\n::\nmath::\nm\n::\n";

#[test]
fn every_prefix_parses_or_is_rejected() {
    for mode in [Mode::Full, Mode::Partial, Mode::Metadata] {
        for end in 0..=SAMPLE.len() {
            let _ = parse(&SAMPLE.as_bytes()[..end], mode);
        }
    }
}

#[test]
fn the_whole_sample_parses() {
    assert!(parse(SAMPLE, Mode::Full).is_ok());
    assert!(parse(SAMPLE, Mode::Metadata).is_ok());
}

#[test]
fn bytes_that_are_not_utf8_still_parse() {
    // Latin-1 `é`, as some old help files have.
    let doc = parse(b"class:: Caf\xe9\nsummary:: x\n", Mode::Full).unwrap();
    assert_eq!(
        doc.children[0].children[0].text.as_deref(),
        Some("Caf\u{fffd}")
    );
}
