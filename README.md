# sclang-lsp

[![CI](https://github.com/jamiehodge/sclang-lsp/actions/workflows/ci.yml/badge.svg)](https://github.com/jamiehodge/sclang-lsp/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/jamiehodge/sclang-lsp)](https://github.com/jamiehodge/sclang-lsp/releases/latest)
[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](LICENSE)

A language server for SuperCollider: completion, hover, goto-definition,
find-references, rename, document and workspace symbols, inlay hints, and
syntax errors as you type.

It reads SuperCollider by parsing it, so it works on code that does not
compile, while sclang is busy, and on a machine with no SuperCollider
installed at all. It never starts sclang and never talks to one.

- [What you get](#what-you-get) — every request, and what each one really answers
- [Install](#install) — VS Code, Claude Code, Neovim, Emacs, Helix
- [Configuration](#configuration) — class library paths, indexing, colour
- [Running code](#running-code) — why that is the editor's job
- [Formatting](#formatting) — an indenter, not a formatter
- [What it does not do](#what-it-does-not-do) — and why each one is deliberate
- [Why you can trust it](#why-you-can-trust-it) — checked against sclang itself

## What you get

Every request the server answers:

| | |
|---|---|
| **Diagnostics** | Syntax errors, per keystroke |
| **Completion** | Parameter names inside a call, then names in scope, then class names. Class-side methods after `Foo.`, inherited ones included; instance methods after `this.`, a literal or a variable whose class is known |
| **Signature help** | The call being typed, with the current parameter marked. `name:` selects its own parameter rather than its position |
| **Hover** | Signature, superclass chain, and the comment above the definition. For a local, what kind of binding it is and its default; for a slot a class inherits, which class it comes from |
| **Goto-definition** | Exact for a class, a local, an inherited class slot, and a receiver whose class is known — a class name, `this`, `super`, a literal, a variable initialised with a constructor; every implementor otherwise |
| **Goto-implementation** | Every class defining a selector; on a class name, its subclasses |
| **Find references** | Exact for a local or a class name; textual for a selector, since dispatch is dynamic |
| **Document highlight** | The other places a name is written in this file, with the declaration marked |
| **Rename** | Function locals and class names — everything whose uses can be enumerated completely |
| **Inlay hints** | The parameter each positional argument fills, where the receiver's class is a fact rather than a convention |
| **Semantic tokens** | Colour from the parse tree: a selector is a method, a name declared as `arg` stays a parameter wherever it appears, and a slot inherited from a superclass is a property. `full`, `range` and `full/delta` |
| **Symbols** | Classes with their methods nested, and across the workspace |
| **Folding** | Regions, class and method bodies, collections and block comments — including the `( … )` idiom indentation folding cannot see |
| **Selection ranges** | Expand-selection, following the real syntax |
| **Formatting** | Indentation on save or over a selection. Leading whitespace only: no line break moves, and columns you aligned by hand stay aligned |

Unsaved edits count immediately: a class that exists only in a buffer is
visible to completion everywhere else. Files that change on disk count too —
the server asks the client to watch `.sc` files, so a `git checkout` or a quark
install does not leave the index describing code that has moved.

## Install

The server is a single binary. VS Code and Claude Code have packaged paths;
everything else points at it directly.

### VS Code

Download the `.vsix` for your platform from the
[releases page](../../releases) and install it:

```bash
code --install-extension sclang-lsp-darwin-arm64.vsix
```

The server is bundled inside, so there is nothing to configure and
SuperCollider itself is only needed if you want to run code. The extension adds
evaluation and a post window too — see [its README](editors/vscode/README.md).

Not on the Marketplace yet. To build it yourself, see
[DEVELOPING.md](editors/vscode/DEVELOPING.md).

> Disable any other SuperCollider extension first. Two extensions contributing
> the `supercollider` language means two servers answering every request.

Indentation on save is off until you ask for it:

```json
"[supercollider]": { "editor.formatOnSave": true }
```

Otherwise it is **Format Document** and **Format Selection**, on whatever your
keyboard shortcuts for those are.

### Claude Code

Claude Code speaks LSP natively, so this is a plugin with no code in it:

```
/plugin marketplace add jamiehodge/sclang-lsp
/plugin install sclang-lsp
```

It needs the binary on `PATH` — see [its README](editors/claude-code). The
request Claude gets most use from is goto-implementation, since "every class
defining this selector" is the honest answer in a dynamically dispatched
language.

### Everything else

Download a binary from the [releases page](../../releases), or build one with
`cargo build --release`. Then point your editor at it for the `supercollider`
language. The server speaks LSP over stdio and ignores arguments it does not
recognise, so clients that pass `--stdio` are fine.

**Neovim** (0.11 or newer):

```lua
vim.filetype.add({ extension = { sc = "supercollider", scd = "supercollider" } })

vim.lsp.config.sclang_lsp = {
  cmd = { "sclang-lsp" },
  filetypes = { "supercollider" },
  root_markers = { ".git" },
}
vim.lsp.enable("sclang_lsp")

vim.api.nvim_create_autocmd("BufWritePre", {
  pattern = { "*.sc", "*.scd" },
  callback = function() vim.lsp.buf.format() end,
})
```

`gq` works on a selection without any of that: Neovim points `formatexpr` at
the server when one offers range formatting.

**Emacs**, with eglot. `sclang-mode` comes from
[scel](https://github.com/supercollider/scel); any major mode will do:

```elisp
(add-to-list 'auto-mode-alist '("\\.scd?\\'" . sclang-mode))
(with-eval-after-load 'eglot
  (add-to-list 'eglot-server-programs '(sclang-mode . ("sclang-lsp"))))
(add-hook 'sclang-mode-hook #'eglot-ensure)

;; Indent on save. `eglot-format` does a region instead.
(add-hook 'sclang-mode-hook
          (lambda () (add-hook 'before-save-hook #'eglot-format-buffer nil t)))
```

**Helix**, in `~/.config/helix/languages.toml`. There is no `grammar` entry
because Helix ships no SuperCollider tree-sitter grammar; what colour you get
depends on whether your Helix build consumes semantic tokens:

```toml
[language-server.sclang-lsp]
command = "sclang-lsp"

[[language]]
name = "supercollider"
scope = "source.supercollider"
file-types = ["sc", "scd"]
comment-token = "//"
language-servers = ["sclang-lsp"]
auto-format = true
```

Helix indents from tree-sitter queries, and ships none for SuperCollider, so
this is the only indentation it has for the language rather than a replacement
for what it was already doing.

Only the VS Code path is exercised here; the three snippets above are offered
untested, and corrections are welcome.

## Configuration

The class library is found automatically, along with `Extensions` and
`downloaded-quarks` — and with whatever `sclang_conf.yaml` adds, which is
where a quark checked out somewhere of your own is named. That file is how
sclang itself is told what to compile, so reading it is what keeps the two
looking at the same code; `excludePaths` and `excludeDefaultPaths` are honoured
with it. To point somewhere else instead — a non-standard install, or a build
tree — pass paths in `initializationOptions`:

```json
{ "classLibraryPaths": ["/path/to/SCClassLibrary"] }
```

Indexing runs in the background, so startup is immediate. The stock class
library takes about half a second, after which requests are well under a
millisecond.

Syntax colouring needs nothing set up. The server offers semantic tokens to
every client, and a client that does not consume them simply never asks; one
that has a grammar of its own layers ours over it rather than replacing it.

## Running code

Not the server's job. It never starts sclang, so nothing it answers can be
delayed or broken by one.

Evaluation, a post window and server control belong to the editor, which is
already managing an sclang for you. The VS Code extension does it in a child
process of its own, and that is also what lets it report **class-library
compile errors** — sclang prints those before any image exists, so only
something holding its output can see them. In Neovim and Emacs, `scnvim` and
`scel` already fill the same role.

## Formatting

An indenter, not a formatter. It rewrites the leading whitespace of a line and
nothing else, so no line break moves and no column you aligned by hand is
disturbed:

```supercollider
Pbind(
    \degree, Pseq([0, 2, 4], inf),
    \dur,    0.25
)
```

That is deliberate rather than unfinished. SuperCollider's two most-written
idioms — a `Pbind`'s key/value pairs and a `SynthDef`'s UGen arguments — are
aligned columns, and every general line-breaking algorithm either collapses
them onto one line or explodes them to one item per line. There is no agreed
SuperCollider style to converge on either, so choosing line breaks would mean
minting one.

A body is one level deeper than the line its bracket opened on, a closing
bracket takes that line exactly, and a line continuing an expression keeps the
offset you gave it — so a hand-aligned continuation still moves with its block.
Tabs or spaces is your editor's setting and never ours.

A file that does not parse is left alone, and says nothing about it. The
diagnostics already report why, and guessing at the indentation of code whose
structure could not be read is how a formatter eats your work.

## What it does not do

**No type inference.** Nothing follows a value through an assignment, out of a
method, or across a call. What is read instead is what the grammar has already
settled: a literal's class, `this` and `super`, and the class a variable was
initialised with. Past that — `x.foo` where `x` came from somewhere the tree
does not say — every class defining `foo` is the honest answer, and narrowing
never replaces that list with nothing.

Inlay hints and keyword-argument completion go one step further again, and stay
silent unless the class is a fact rather than a convention. Both render as
though they were written in the source, and `Foo.new` returning a `Foo` is a
convention a class is free to break.

**No references for an inherited class slot.** Hover and goto will tell you
where `pattern` is declared; find-references will not enumerate who reads it.
Its uses are ordinary identifiers spread across every subclass in the
workspace, and the occurrence index records only class names and selectors. An
answer covering the open buffer alone would read as complete and would not be.

**Rename refuses methods**, and says why. `.play` is dispatched at run time, so
nothing distinguishes one class's `play` from another's, and rewriting every
`.play` in a workspace would break the classes that were not meant. Instance
variables are refused for the same kind of reason: `var <count` generates
`count` and `count_`, and subclasses inherit both. Find-references has no such
restriction, because a wrong row in a list costs a glance rather than a working
program.

**No `~envir` completion.** Environment variables live in a running image and
nothing static can enumerate them. This is the one real capability given up by
never contacting sclang, and it is worth the exchange.

**Nothing happens as you type.** Formatting is a request an editor makes on
save or on demand; LSP has no way for a server to contribute indentation rules,
so the Enter key belongs to VS Code's `language-configuration.json` and to
tree-sitter `indents.scm` in Helix, Zed and Neovim whatever this server knows.
Vim's `=` and Emacs' TAB do not route through it either. That is a second
reason a tree-sitter grammar is worth having.

No SCDoc rendering yet.

## Why you can trust it

Both halves of the front end are derived from SuperCollider's own, in
`lang/LangSource`: the lexer is a port of `PyrLexer.cpp`, and the parser comes
from the bison grammar in `Bison/lang11d`. A grammar maintained separately
drifts, and the drift is invisible — it produces trees that parse without error
but are wrong.

That claim is checked rather than asserted, against sclang itself:

| | |
|---|---|
| Tokens, against upstream's `sc_lexer` | 680 / 680 files agree exactly |
| Symbols, against the compiled class library | 14,484 / 14,490 methods match exactly |
| Expression structure, against sclang's parse dump | 9,731 / 9,731 method bodies identical |
| Every `.sc` class file in the corpus | parses |

[CONFORMANCE.md](CONFORMANCE.md) has the detail, including the five bugs in
sclang's own parse-tree dumper that had to be fixed before the last of those
meant anything.

## Documentation

- [CONFORMANCE.md](CONFORMANCE.md) — how the front end is derived and verified
- [ARCHITECTURE.md](ARCHITECTURE.md) — the design decisions behind the topology
- [CONTRIBUTING.md](CONTRIBUTING.md) — building, testing, and where the bar is
- [CHANGELOG.md](CHANGELOG.md) — what changed and when
- [`editors/vscode`](editors/vscode) — the extension, and running code
- [`editors/claude-code`](editors/claude-code) — the Claude Code plugin

## License

GPL-3.0-or-later, matching SuperCollider, since this is a derived work of its
front end.
