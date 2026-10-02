//! The SCDoc grammar, ported from `SCDoc/SCDoc.y` in the SuperCollider source
//! tree, as of 3.14.1.
//!
//! The bison grammar is LALR(1), but every choice in it is settled by one
//! token of lookahead from disjoint sets — in particular nothing that can
//! follow a `body` can begin one — so it reads as recursive descent without
//! changing the language. Each function below names the productions it
//! implements, and builds the node the corresponding semantic action does.
//!
//! The intermediate nodes bison builds and then dissolves —
//! `(SECTIONBODY)`, `(SUBSUBSECTIONS)` and the rest, always consumed by
//! `doc_node_make_take_children` — are plain `Vec`s here.
//!
//! Upstream has no error recovery worth the name: `start: document error`
//! discards the tree, so any syntax error means no document at all. That is
//! kept. These files are read from an installation, where upstream rendering
//! nothing is the behaviour to match, not to improve on.

use crate::lexer::{Tok, Token};
use crate::tree::{Id, RawNode};
use crate::{Error, Mode};

type Result<T> = std::result::Result<T, Error>;

pub(crate) fn parse(src: &[u8], tokens: &[Token], mode: Mode) -> Result<RawNode> {
    let mut p = Parser {
        src,
        tokens,
        pos: 0,
        method_type: Id::Method,
    };
    let doc = p.document(mode)?;
    p.expect(Tok::End)?;
    Ok(doc)
}

struct Parser<'a> {
    src: &'a [u8],
    tokens: &'a [Token],
    pos: usize,
    /// The global `method_type`: which kind of node `method::` makes, set by
    /// the most recent section tag and never reset by anything else.
    method_type: Id,
}

impl Parser<'_> {
    fn peek(&self) -> Tok {
        self.tokens[self.pos].kind
    }

    fn at(&self, kind: Tok) -> bool {
        self.peek() == kind
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos];
        // `End` is returned forever, as flex does at end of input.
        if t.kind != Tok::End {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, kind: Tok) -> bool {
        let found = self.at(kind);
        if found {
            self.bump();
        }
        found
    }

    fn expect(&mut self, kind: Tok) -> Result<()> {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.unexpected())
        }
    }

    fn unexpected(&self) -> Error {
        let t = &self.tokens[self.pos];
        Error {
            offset: t.start,
            message: format!("syntax error, unexpected {:?}", t.kind),
        }
    }

    fn error(&self, message: &str) -> Error {
        Error {
            offset: self.tokens[self.pos.saturating_sub(1)].start,
            message: message.to_string(),
        }
    }

    // ---- document ---------------------------------------------------------

    /// `document: START_FULL eateol dochead optsections`
    /// `        | START_PARTIAL sections`
    /// `        | START_METADATA dochead optsections`
    fn document(&mut self, mode: Mode) -> Result<RawNode> {
        match mode {
            Mode::Full | Mode::Metadata => {
                if mode == Mode::Full {
                    // eateol: eol | /* empty */
                    self.opt_eol();
                }
                let head = self.dochead()?;
                // optsections: sections | /* empty */
                let body = if self.at(Tok::End) {
                    Vec::new()
                } else {
                    self.sections()?
                };
                Ok(RawNode::new(
                    Id::Document,
                    None,
                    vec![head, RawNode::new(Id::Body, None, body)],
                ))
            }
            Mode::Partial => Ok(RawNode::new(Id::Body, None, self.sections()?)),
        }
    }

    /// `dochead: dochead headline | headline`
    fn dochead(&mut self) -> Result<RawNode> {
        let mut lines = vec![self.headline()?];
        while matches!(
            self.peek(),
            Tok::Class | Tok::Title | Tok::Summary | Tok::Redirect | Tok::Categories | Tok::Related
        ) {
            lines.push(self.headline()?);
        }
        Ok(RawNode::new(Id::Header, None, lines))
    }

    /// `headline: headtag words2 eol`
    /// `        | CATEGORIES commalist eol`
    /// `        | RELATED commalist eol`
    fn headline(&mut self) -> Result<RawNode> {
        let node = match self.peek() {
            Tok::Class | Tok::Title => self.tagged_words2(Id::Title)?,
            Tok::Summary => self.tagged_words2(Id::Summary)?,
            Tok::Redirect => self.tagged_words2(Id::Redirect)?,
            Tok::Categories => {
                self.bump();
                RawNode::new(Id::Categories, None, self.commalist()?)
            }
            Tok::Related => {
                self.bump();
                RawNode::new(Id::Related, None, self.commalist()?)
            }
            _ => return Err(self.unexpected()),
        };
        self.eol()?;
        Ok(node)
    }

    fn tagged_words2(&mut self, id: Id) -> Result<RawNode> {
        self.bump();
        Ok(RawNode::new(id, Some(self.words2()?), Vec::new()))
    }

    // ---- sections ---------------------------------------------------------

    /// `sections: sections section | section | subsubsections`
    ///
    /// The children of the `BODY` node; `subsubsections` is only allowed
    /// first, as the text before the first section.
    fn sections(&mut self) -> Result<Vec<RawNode>> {
        let mut out = Vec::new();
        if self.at_section() {
            out.push(self.section()?);
        } else if self.at_subsubsections() {
            out = self.subsubsections()?;
        } else {
            return Err(self.unexpected());
        }
        while self.at_section() {
            out.push(self.section()?);
        }
        Ok(out)
    }

    fn at_section(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Section
                | Tok::ClassMethods
                | Tok::InstanceMethods
                | Tok::Description
                | Tok::Examples
        )
    }

    /// `section: SECTION words2 eol optsubsections | sectiontag optsubsections`
    fn section(&mut self) -> Result<RawNode> {
        let (id, text) = match self.bump().kind {
            Tok::Section => {
                // The mid-rule action, which runs before `words2`.
                self.method_type = Id::Method;
                let words = self.words2()?;
                self.eol()?;
                (Id::Section, Some(words))
            }
            // sectiontag, whose actions also set `method_type`.
            Tok::ClassMethods => {
                self.method_type = Id::CMethod;
                (Id::ClassMethods, None)
            }
            Tok::InstanceMethods => {
                self.method_type = Id::IMethod;
                (Id::InstanceMethods, None)
            }
            Tok::Description => {
                self.method_type = Id::Method;
                (Id::Description, None)
            }
            Tok::Examples => {
                self.method_type = Id::Method;
                (Id::Examples, None)
            }
            _ => unreachable!("at_section checked the token"),
        };
        Ok(RawNode::new(id, text, self.opt_subsections()?))
    }

    /// `optsubsections: subsections | /* empty */`
    /// `subsections: subsections subsection | subsection | subsubsections`
    fn opt_subsections(&mut self) -> Result<Vec<RawNode>> {
        let mut out = Vec::new();
        if self.at(Tok::Subsection) {
            out.push(self.subsection()?);
        } else if self.at_subsubsections() {
            out = self.subsubsections()?;
        } else {
            return Ok(out);
        }
        while self.at(Tok::Subsection) {
            out.push(self.subsection()?);
        }
        Ok(out)
    }

    /// `subsection: SUBSECTION words2 eol optsubsubsections`
    fn subsection(&mut self) -> Result<RawNode> {
        self.expect(Tok::Subsection)?;
        let words = self.words2()?;
        self.eol()?;
        // optsubsubsections: subsubsections | /* empty */
        let children = if self.at_subsubsections() {
            self.subsubsections()?
        } else {
            Vec::new()
        };
        Ok(RawNode::new(Id::Subsection, Some(words), children))
    }

    fn at_subsubsections(&self) -> bool {
        self.at_subsubsection() || self.at_body()
    }

    fn at_subsubsection(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Subsubsection | Tok::Method | Tok::CopyMethod | Tok::Private
        )
    }

    /// `subsubsections: subsubsections subsubsection | subsubsection | body`
    fn subsubsections(&mut self) -> Result<Vec<RawNode>> {
        let mut out = if self.at_body() {
            self.body()?
        } else {
            vec![self.subsubsection()?]
        };
        while self.at_subsubsection() {
            out.push(self.subsubsection()?);
        }
        Ok(out)
    }

    /// `subsubsection: SUBSUBSECTION words2 eol optbody`
    /// `             | METHOD methnames optMETHODARGS eol methodbody`
    /// `             | COPYMETHOD words eol`
    /// `             | PRIVATE commalist eoleof`
    fn subsubsection(&mut self) -> Result<RawNode> {
        match self.bump().kind {
            Tok::Subsubsection => {
                let words = self.words2()?;
                self.eol()?;
                Ok(RawNode::new(
                    Id::Subsubsection,
                    Some(words),
                    self.opt_body()?,
                ))
            }
            Tok::Method => {
                let names = self.methnames()?;
                // optMETHODARGS
                let args = if self.at(Tok::MethodArgs) {
                    let args = self.bump().text(self.src).to_vec();
                    if self.method_type != Id::Method {
                        return Err(self.error(
                            "METHOD argument string is not allowed inside CLASSMETHODS or INSTANCEMETHODS",
                        ));
                    }
                    Some(args)
                } else {
                    None
                };
                self.eol()?;
                let body = self.methodbody()?;
                Ok(RawNode::new(self.method_type, args, vec![names, body]))
            }
            Tok::CopyMethod => {
                let words = self.words()?;
                if !words.contains(&b' ') {
                    return Err(
                        self.error("COPYMETHOD requires 2 arguments (class name and method name)")
                    );
                }
                self.eol()?;
                let id = match self.method_type {
                    Id::CMethod => Id::CCopyMethod,
                    Id::IMethod => Id::ICopyMethod,
                    _ => Id::CopyMethod,
                };
                Ok(RawNode::new(id, Some(words), Vec::new()))
            }
            Tok::Private => {
                let names = self.commalist()?;
                self.eoleof()?;
                let id = match self.method_type {
                    Id::CMethod => Id::CPrivate,
                    _ => Id::IPrivate,
                };
                Ok(RawNode::new(id, None, names))
            }
            _ => unreachable!("at_subsubsection checked the token"),
        }
    }

    /// `methnames: methnames COMMA methodname | methodname`
    fn methnames(&mut self) -> Result<RawNode> {
        let mut names = vec![self.methodname()?];
        while self.eat(Tok::Comma) {
            names.push(self.methodname()?);
        }
        Ok(RawNode::new(Id::MethodNames, None, names))
    }

    /// `methodname: METHODNAME`, whose action drops a setter's trailing `_`
    /// with a warning.
    fn methodname(&mut self) -> Result<RawNode> {
        if !self.at(Tok::MethodName) {
            return Err(self.unexpected());
        }
        let mut name = self.bump().text(self.src).to_vec();
        if name.last() == Some(&b'_') {
            name.pop();
        }
        Ok(RawNode::new(Id::String, Some(name), Vec::new()))
    }

    /// `methodbody: optbody optargs optreturns optdiscussion`
    fn methodbody(&mut self) -> Result<RawNode> {
        let mut children = self.opt_body()?;
        // optargs: args | /* empty */
        if self.at(Tok::Argument) {
            let mut args = Vec::new();
            while self.at(Tok::Argument) {
                args.push(self.arg()?);
            }
            children.push(RawNode::new(Id::Arguments, None, args));
        }
        // optreturns: RETURNS body | /* empty */
        if self.eat(Tok::Returns) {
            children.push(RawNode::new(Id::Returns, None, self.body()?));
        }
        // optdiscussion: DISCUSSION body | /* empty */
        if self.eat(Tok::Discussion) {
            children.push(RawNode::new(Id::Discussion, None, self.body()?));
        }
        Ok(RawNode::new(Id::MethodBody, None, children))
    }

    /// `arg: ARGUMENT words eol optbody | ARGUMENT eol body`
    fn arg(&mut self) -> Result<RawNode> {
        self.expect(Tok::Argument)?;
        if self.at_eol() {
            self.bump();
            return Ok(RawNode::new(Id::Argument, None, self.body()?));
        }
        let words = self.words()?;
        self.eol()?;
        Ok(RawNode::new(Id::Argument, Some(words), self.opt_body()?))
    }

    // ---- body -------------------------------------------------------------

    fn at_body(&self) -> bool {
        self.at_prose()
            || matches!(
                self.peek(),
                Tok::Warning
                    | Tok::Note
                    | Tok::List
                    | Tok::Tree
                    | Tok::NumberedList
                    | Tok::Table
                    | Tok::DefinitionList
                    | Tok::CodeBlock
                    | Tok::TeletypeBlock
                    | Tok::MathBlock
                    | Tok::ClassTree
                    | Tok::Keyword
                    | Tok::EmptyLines
                    | Tok::Image
            )
    }

    fn at_prose(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Text
                | Tok::Comma
                | Tok::Url
                | Tok::Link
                | Tok::Strong
                | Tok::Soft
                | Tok::Emphasis
                | Tok::Code
                | Tok::Teletype
                | Tok::Math
                | Tok::Anchor
                | Tok::Footnote
                | Tok::Newline
        )
    }

    /// `optbody: body | /* empty */`
    fn opt_body(&mut self) -> Result<Vec<RawNode>> {
        if self.at_body() {
            self.body()
        } else {
            Ok(Vec::new())
        }
    }

    /// `body: blockA | blockB`, where the two blocks alternate body elements
    /// with runs of prose. The upshot is a non-empty sequence of both, with
    /// prose maximal. `EMPTYLINES` is a body element that makes no node, which
    /// is how two paragraphs become two `PROSE` nodes.
    fn body(&mut self) -> Result<Vec<RawNode>> {
        if !self.at_body() {
            return Err(self.unexpected());
        }
        let mut out = Vec::new();
        while self.at_body() {
            if self.at_prose() {
                out.push(self.prose()?);
            } else if let Some(node) = self.bodyelem()? {
                out.push(node);
            }
        }
        Ok(out)
    }

    /// `bodyelem`, which is `None` for `EMPTYLINES`.
    fn bodyelem(&mut self) -> Result<Option<RawNode>> {
        let kind = self.bump().kind;
        let node = match kind {
            // rangetag body TAGSYM
            Tok::Warning | Tok::Note => {
                let id = if kind == Tok::Warning {
                    Id::Warning
                } else {
                    Id::Note
                };
                let body = self.body()?;
                self.expect(Tok::TagSym)?;
                RawNode::new(id, None, body)
            }
            // listtag listbody TAGSYM
            // listbody: listbody HASHES body | HASHES body
            Tok::List | Tok::Tree | Tok::NumberedList => {
                let id = match kind {
                    Tok::List => Id::List,
                    Tok::Tree => Id::Tree,
                    _ => Id::NumberedList,
                };
                let mut items = Vec::new();
                loop {
                    self.expect(Tok::Hashes)?;
                    items.push(RawNode::new(Id::Item, None, self.body()?));
                    if !self.at(Tok::Hashes) {
                        break;
                    }
                }
                self.expect(Tok::TagSym)?;
                RawNode::new(id, None, items)
            }
            // TABLE tablebody TAGSYM
            // tablebody: tablebody tablerow | tablerow
            // tablerow: HASHES tablecells
            // tablecells: tablecells BARS optbody | optbody
            Tok::Table => {
                let mut rows = Vec::new();
                loop {
                    self.expect(Tok::Hashes)?;
                    let mut cells = vec![RawNode::new(Id::TabCol, None, self.opt_body()?)];
                    while self.eat(Tok::Bars) {
                        cells.push(RawNode::new(Id::TabCol, None, self.opt_body()?));
                    }
                    rows.push(RawNode::new(Id::TabRow, None, cells));
                    if !self.at(Tok::Hashes) {
                        break;
                    }
                }
                self.expect(Tok::TagSym)?;
                RawNode::new(Id::Table, None, rows)
            }
            // DEFINITIONLIST deflistbody TAGSYM
            // deflistbody: deflistbody deflistrow | deflistrow
            // deflistrow: defterms BARS optbody
            // defterms: defterms HASHES body | HASHES body
            Tok::DefinitionList => {
                let mut rows = Vec::new();
                loop {
                    let mut item = Vec::new();
                    loop {
                        self.expect(Tok::Hashes)?;
                        item.push(RawNode::new(Id::Term, None, self.body()?));
                        if !self.at(Tok::Hashes) {
                            break;
                        }
                    }
                    self.expect(Tok::Bars)?;
                    item.push(RawNode::new(Id::Definition, None, self.opt_body()?));
                    rows.push(RawNode::new(Id::DefListItem, None, item));
                    if !self.at(Tok::Hashes) {
                        break;
                    }
                }
                self.expect(Tok::TagSym)?;
                RawNode::new(Id::DefinitionList, None, rows)
            }
            // blocktag wordsnl TAGSYM
            Tok::CodeBlock | Tok::TeletypeBlock | Tok::MathBlock => {
                let id = match kind {
                    Tok::CodeBlock => Id::CodeBlock,
                    Tok::TeletypeBlock => Id::TeletypeBlock,
                    _ => Id::MathBlock,
                };
                let text = self.wordsnl()?;
                self.expect(Tok::TagSym)?;
                RawNode::new(id, Some(text), Vec::new())
            }
            // CLASSTREE words eoleof
            Tok::ClassTree => {
                let words = self.words()?;
                self.eoleof()?;
                RawNode::new(Id::ClassTree, Some(words), Vec::new())
            }
            // KEYWORD commalist eoleof
            Tok::Keyword => {
                let words = self.commalist()?;
                self.eoleof()?;
                RawNode::new(Id::Keyword, None, words)
            }
            Tok::EmptyLines => return Ok(None),
            // IMAGE words2 TAGSYM
            Tok::Image => {
                let words = self.words2()?;
                self.expect(Tok::TagSym)?;
                RawNode::new(Id::Image, Some(words), Vec::new())
            }
            _ => unreachable!("at_body checked the token"),
        };
        Ok(Some(node))
    }

    /// `prose: prose proseelem | proseelem`
    ///
    /// Bison makes one `TEXT` node per word and the fixup pass joins adjacent
    /// ones. Joining them here instead is the same result for a fraction of
    /// the allocation. Line breaks are left for the fixup, which drops a
    /// trailing one before it joins anything.
    fn prose(&mut self) -> Result<RawNode> {
        let mut elems: Vec<RawNode> = Vec::new();
        while self.at_prose() {
            if matches!(self.peek(), Tok::Text | Tok::Comma) {
                if let Some(RawNode {
                    id: Id::Text,
                    text: Some(prev),
                    ..
                }) = elems.last_mut()
                {
                    let t = self.bump();
                    prev.extend_from_slice(t.text(self.src));
                    continue;
                }
            }
            elems.push(self.proseelem()?);
        }
        Ok(RawNode::new(Id::Prose, None, elems))
    }

    /// `proseelem: anyword | URL | inlinetag words TAGSYM`
    /// `         | FOOTNOTE body TAGSYM | NEWLINE`
    fn proseelem(&mut self) -> Result<RawNode> {
        let t = self.bump();
        let (kind, text) = (t.kind, t.text(self.src).to_vec());
        let inline = |id| (id, None, Vec::new());
        let (id, text, children) = match kind {
            Tok::Text | Tok::Comma => (Id::Text, Some(text), Vec::new()),
            Tok::Url => (Id::Link, Some(text), Vec::new()),
            Tok::Newline => inline(Id::Nl),
            Tok::Footnote => {
                let body = self.body()?;
                self.expect(Tok::TagSym)?;
                (Id::Footnote, None, body)
            }
            _ => {
                let id = match kind {
                    Tok::Link => Id::Link,
                    Tok::Strong => Id::Strong,
                    Tok::Soft => Id::Soft,
                    Tok::Emphasis => Id::Emphasis,
                    Tok::Code => Id::Code,
                    Tok::Teletype => Id::Teletype,
                    Tok::Math => Id::Math,
                    Tok::Anchor => Id::Anchor,
                    _ => unreachable!("at_prose checked the token"),
                };
                let words = self.words()?;
                self.expect(Tok::TagSym)?;
                (id, Some(words), Vec::new())
            }
        };
        Ok(RawNode::new(id, text, children))
    }

    // ---- words ------------------------------------------------------------

    /// Concatenate a run of tokens drawn from `ok`, as `strmerge` does. At
    /// least one is required.
    fn run(&mut self, ok: impl Fn(Tok) -> bool) -> Result<Vec<u8>> {
        if !ok(self.peek()) {
            return Err(self.unexpected());
        }
        let mut out = Vec::new();
        while ok(self.peek()) {
            out.extend_from_slice(self.bump().text(self.src));
        }
        Ok(out)
    }

    /// `words: words anyword | anyword`, where `anyword: TEXT | COMMA`.
    fn words(&mut self) -> Result<Vec<u8>> {
        self.run(|k| matches!(k, Tok::Text | Tok::Comma))
    }

    /// `words2: words2 anywordurl | anywordurl`
    fn words2(&mut self) -> Result<Vec<u8>> {
        self.run(|k| matches!(k, Tok::Text | Tok::Comma | Tok::Url))
    }

    /// `wordsnl: wordsnl anywordnl | anywordnl`, with every `eol` read as a
    /// single `"\n"` whatever it spanned.
    fn wordsnl(&mut self) -> Result<Vec<u8>> {
        let ok = |k| matches!(k, Tok::Text | Tok::Comma | Tok::Newline | Tok::EmptyLines);
        if !ok(self.peek()) {
            return Err(self.unexpected());
        }
        let mut out = Vec::new();
        while ok(self.peek()) {
            let t = self.bump();
            match t.kind {
                Tok::Newline | Tok::EmptyLines => out.push(b'\n'),
                _ => out.extend_from_slice(t.text(self.src)),
            }
        }
        Ok(out)
    }

    /// `commalist: commalist COMMA nocommawords | nocommawords`, where
    /// `nocommawords` is a run of `TEXT` and `URL`.
    fn commalist(&mut self) -> Result<Vec<RawNode>> {
        let mut items = Vec::new();
        loop {
            let words = self.run(|k| matches!(k, Tok::Text | Tok::Url))?;
            items.push(RawNode::new(Id::String, Some(words), Vec::new()));
            if !self.eat(Tok::Comma) {
                return Ok(items);
            }
        }
    }

    // ---- line ends --------------------------------------------------------

    fn at_eol(&self) -> bool {
        matches!(self.peek(), Tok::Newline | Tok::EmptyLines)
    }

    /// `eol: NEWLINE | EMPTYLINES`
    fn eol(&mut self) -> Result<()> {
        if self.at_eol() {
            self.bump();
            Ok(())
        } else {
            Err(self.unexpected())
        }
    }

    fn opt_eol(&mut self) {
        if self.at_eol() {
            self.bump();
        }
    }

    /// `eoleof: eol | END`. `END` is not consumed, since the lexer goes on
    /// returning it and `start` needs one more.
    fn eoleof(&mut self) -> Result<()> {
        if self.at(Tok::End) {
            Ok(())
        } else {
            self.eol()
        }
    }
}
