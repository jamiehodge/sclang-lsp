# sclang-lsp

A language server for SuperCollider.

Completion that knows the class library. Hover with real signatures and the
comment above the definition. Goto-definition, find-references, rename,
document and workspace symbols, inlay hints, and syntax errors as you type.

It reads SuperCollider by parsing it, so it works on code that does not
compile, while sclang is busy, and on a machine with no SuperCollider
installed at all. It never starts sclang and never talks to one.

## What you get

| | |
|---|---|
| **Diagnostics** | Syntax errors, per keystroke |
| **Completion** | Parameter names inside a call, then names in scope, then class names. Class-side methods after `Foo.`, inherited ones included |
| **Signature help** | The call being typed, with the current parameter marked. `name:` selects its own parameter rather than its position |
| **Hover** | Signature, superclass chain, and the comment above the definition. For a local, what kind of binding it is and its default |
| **Goto-definition** | Exact for a class, a class receiver or a local; every implementor otherwise |
| **Find references** | Exact for a local or a class name; textual for a selector, since dispatch is dynamic |
| **Rename** | Function locals and class names — everything whose uses can be enumerated completely |
| **Inlay hints** | The parameter each positional argument fills |
| **Symbols** | Classes with their methods nested, and across the workspace |
| **Selection ranges** | Expand-selection, following the real syntax |

Unsaved edits count immediately: a class that exists only in a buffer is
visible to completion everywhere else.

## Install

### VS Code

The extension lives in [`editors/vscode`](editors/vscode) and bundles the
server. It is not on the Marketplace yet, so build the `.vsix`:

```bash
cargo build --release
cd editors/vscode && npm install && npm run package
code --install-extension sclang-lsp-*.vsix
```

It adds evaluation and a post window too — see
[its README](editors/vscode/README.md).

> Disable any other SuperCollider extension first. Two extensions contributing
> the `supercollider` language means two servers answering every request.

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
```

**Emacs**, with eglot. `sclang-mode` comes from
[scel](https://github.com/supercollider/scel); any major mode will do:

```elisp
(add-to-list 'auto-mode-alist '("\\.scd?\\'" . sclang-mode))
(with-eval-after-load 'eglot
  (add-to-list 'eglot-server-programs '(sclang-mode . ("sclang-lsp"))))
(add-hook 'sclang-mode-hook #'eglot-ensure)
```

**Helix**, in `~/.config/helix/languages.toml`. Omitting `grammar` means no
syntax highlighting, since Helix ships no SuperCollider grammar:

```toml
[language-server.sclang-lsp]
command = "sclang-lsp"

[[language]]
name = "supercollider"
scope = "source.supercollider"
file-types = ["sc", "scd"]
comment-token = "//"
language-servers = ["sclang-lsp"]
```

Only the VS Code path is exercised here; the three snippets above are offered
untested, and corrections are welcome.

The class library is found automatically, along with `Extensions` and
`downloaded-quarks`. To point somewhere else — a non-standard install, or a
build tree — pass paths in `initializationOptions`:

```json
{ "classLibraryPaths": ["/path/to/SCClassLibrary"] }
```

Indexing runs in the background, so startup is immediate. The stock class
library takes about half a second, after which requests are well under a
millisecond.

## Running code

Not the server's job. It never starts sclang, so nothing it answers can be
delayed or broken by one.

Evaluation, a post window and server control belong to the editor, which is
already managing an sclang for you. The VS Code extension does it in a child
process of its own, and that is also what lets it report **class-library
compile errors** — sclang prints those before any image exists, so only
something holding its output can see them. In Neovim and Emacs, `scnvim` and
`scel` already fill the same role.

## What it does not do

**No type inference.** Without types, `x.foo` offers every class defining
`foo`, which is the honest answer. Inlay hints and keyword-argument completion
go further and stay silent unless the receiver is a literal class name: both
render as though they were in the source, and a guess there would read as a
fact.

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

No SCDoc rendering, no semantic tokens, no document highlight yet.

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
- [`editors/vscode`](editors/vscode) — the extension, and running code

## License

GPL-3.0-or-later, matching SuperCollider, since this is a derived work of its
front end.
