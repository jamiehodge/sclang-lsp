//! The document tree, and the pass upstream runs over it after parsing.
//!
//! The node shape is `DocNode` from `SCDoc/SCDoc.h` — an id, optional text,
//! and children — which is also what sclang hands the class library as
//! `SCDocNode`. Keeping that shape, rather than a typed tree, is what lets the
//! oracle compare against upstream's own dump byte for byte.

/// A node kind. The names are SCDoc's own, as [`Id::as_str`] gives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Id {
    Document,
    Header,
    /// `class::` and `title::` both produce this.
    Title,
    Summary,
    Redirect,
    Categories,
    Related,
    /// An item of a comma-separated list, or one of a method's names.
    String,
    Body,
    Section,
    Subsection,
    /// Added in 3.14.
    Subsubsection,
    Description,
    ClassMethods,
    InstanceMethods,
    Examples,
    /// `method::` outside `classmethods::` and `instancemethods::`.
    Method,
    /// `method::` under `classmethods::`.
    CMethod,
    /// `method::` under `instancemethods::`.
    IMethod,
    MethodNames,
    MethodBody,
    Arguments,
    Argument,
    Returns,
    Discussion,
    CopyMethod,
    CCopyMethod,
    ICopyMethod,
    CPrivate,
    IPrivate,
    Prose,
    Text,
    /// A line break inside prose, before [`fixup`] folds it into text.
    Nl,
    Link,
    Strong,
    Soft,
    Emphasis,
    Code,
    Teletype,
    /// Added in 3.14.
    Math,
    Anchor,
    Footnote,
    CodeBlock,
    TeletypeBlock,
    /// Added in 3.14.
    MathBlock,
    List,
    Tree,
    NumberedList,
    Item,
    Warning,
    Note,
    Table,
    TabRow,
    TabCol,
    DefinitionList,
    DefListItem,
    Term,
    Definition,
    ClassTree,
    Keyword,
    Image,
}

impl Id {
    /// The name upstream gives this kind, and sclang's `SCDocNode.id`.
    pub fn as_str(self) -> &'static str {
        match self {
            Id::Document => "DOCUMENT",
            Id::Header => "HEADER",
            Id::Title => "TITLE",
            Id::Summary => "SUMMARY",
            Id::Redirect => "REDIRECT",
            Id::Categories => "CATEGORIES",
            Id::Related => "RELATED",
            Id::String => "STRING",
            Id::Body => "BODY",
            Id::Section => "SECTION",
            Id::Subsection => "SUBSECTION",
            Id::Subsubsection => "SUBSUBSECTION",
            Id::Description => "DESCRIPTION",
            Id::ClassMethods => "CLASSMETHODS",
            Id::InstanceMethods => "INSTANCEMETHODS",
            Id::Examples => "EXAMPLES",
            Id::Method => "METHOD",
            Id::CMethod => "CMETHOD",
            Id::IMethod => "IMETHOD",
            Id::MethodNames => "METHODNAMES",
            Id::MethodBody => "METHODBODY",
            Id::Arguments => "ARGUMENTS",
            Id::Argument => "ARGUMENT",
            Id::Returns => "RETURNS",
            Id::Discussion => "DISCUSSION",
            Id::CopyMethod => "COPYMETHOD",
            Id::CCopyMethod => "CCOPYMETHOD",
            Id::ICopyMethod => "ICOPYMETHOD",
            Id::CPrivate => "CPRIVATE",
            Id::IPrivate => "IPRIVATE",
            Id::Prose => "PROSE",
            Id::Text => "TEXT",
            Id::Nl => "NL",
            Id::Link => "LINK",
            Id::Strong => "STRONG",
            Id::Soft => "SOFT",
            Id::Emphasis => "EMPHASIS",
            Id::Code => "CODE",
            Id::Teletype => "TELETYPE",
            Id::Math => "MATH",
            Id::Anchor => "ANCHOR",
            Id::Footnote => "FOOTNOTE",
            Id::CodeBlock => "CODEBLOCK",
            Id::TeletypeBlock => "TELETYPEBLOCK",
            Id::MathBlock => "MATHBLOCK",
            Id::List => "LIST",
            Id::Tree => "TREE",
            Id::NumberedList => "NUMBEREDLIST",
            Id::Item => "ITEM",
            Id::Warning => "WARNING",
            Id::Note => "NOTE",
            Id::Table => "TABLE",
            Id::TabRow => "TABROW",
            Id::TabCol => "TABCOL",
            Id::DefinitionList => "DEFINITIONLIST",
            Id::DefListItem => "DEFLISTITEM",
            Id::Term => "TERM",
            Id::Definition => "DEFINITION",
            Id::ClassTree => "CLASSTREE",
            Id::Keyword => "KEYWORD",
            Id::Image => "IMAGE",
        }
    }
}

/// A node of a parsed help file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: Id,
    pub text: Option<String>,
    pub children: Vec<Node>,
}

impl Node {
    /// The first child with the given id.
    pub fn child(&self, id: Id) -> Option<&Node> {
        self.children.iter().find(|c| c.id == id)
    }

    /// The tree as upstream's `doc_node_dump` prints it. The oracle compares
    /// these, so the format is upstream's to the byte.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        dump(self, &mut Vec::new(), &mut out);
        out
    }
}

/// Text accumulated as bytes, because the lexer splits multi-byte characters
/// and only the parser's concatenation puts them back together.
#[derive(Debug, Clone)]
pub(crate) struct RawNode {
    pub id: Id,
    pub text: Option<Vec<u8>>,
    pub children: Vec<RawNode>,
}

impl RawNode {
    pub fn new(id: Id, text: Option<Vec<u8>>, children: Vec<RawNode>) -> Self {
        RawNode { id, text, children }
    }

    pub fn into_node(self) -> Node {
        Node {
            id: self.id,
            text: self.text.map(|t| String::from_utf8_lossy(&t).into_owned()),
            children: self.children.into_iter().map(RawNode::into_node).collect(),
        }
    }
}

/// `doc_node_fixup_tree`, which `scdoc_parse_file` runs on every tree it
/// returns: trailing whitespace off every text but prose, one trailing line
/// break dropped, and adjacent prose words and line breaks joined into runs.
pub(crate) fn fixup(n: &mut RawNode) {
    if n.id != Id::Text {
        if let Some(text) = &mut n.text {
            strip_trailing_ws(text);
        }
    }
    if n.children.is_empty() {
        return;
    }
    if n.children.last().is_some_and(|c| c.id == Id::Nl) {
        n.children.pop();
    }

    let mut kept: Vec<RawNode> = Vec::with_capacity(n.children.len());
    for mut child in std::mem::take(&mut n.children) {
        let joins = matches!(child.id, Id::Text | Id::Nl);
        match kept.last_mut() {
            Some(last) if joins && last.id == Id::Text => {
                let text = last.text.get_or_insert_with(Vec::new);
                match child.id {
                    Id::Nl => text.push(b' '),
                    _ => text.extend(child.text.unwrap_or_default()),
                }
            }
            _ => {
                fixup(&mut child);
                kept.push(child);
            }
        }
    }
    n.children = kept;
}

/// `striptrailingws`. The loop condition is `--s2 > s`, so the first byte is
/// never removed, even when it is whitespace.
fn strip_trailing_ws(s: &mut Vec<u8>) {
    while s.len() > 1 && matches!(s.last(), Some(b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')) {
        s.pop();
    }
}

fn dump(n: &Node, last: &mut Vec<bool>, out: &mut String) {
    if let Some((&is_last, outer)) = last.split_last() {
        for &l in outer {
            out.push_str(if l { "    " } else { "|   " });
        }
        out.push_str(if is_last { "`-- " } else { "|-- " });
    }
    out.push_str(n.id.as_str());
    if let Some(text) = &n.text {
        out.push_str(" \"");
        out.push_str(text);
        out.push('"');
    }
    out.push('\n');
    for (i, child) in n.children.iter().enumerate() {
        last.push(i + 1 == n.children.len());
        dump(child, last, out);
        last.pop();
    }
}
