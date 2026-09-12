# SuperCollider for VS Code, via `sclang-lsp`

Language features served by [`sclang-lsp`](../../README.md): diagnostics as you
type, completion, hover, goto-definition, and document and workspace symbols.

The server parses SuperCollider itself, so all of it works with sclang absent,
broken, or busy — including on a class library that does not compile.

## Running it locally

From the repository root:

```bash
cargo build --release
```

Then, in this directory:

```bash
npm install
npm run compile
```

Open `editors/vscode` in VS Code and press <kbd>F5</kbd>. That launches an
Extension Development Host with the extension loaded; open any `.sc` or `.scd`
file in it.

No configuration is needed while developing: the extension looks for
`target/release/sclang-lsp` (then `target/debug`) in the checkout it lives in
before falling back to `PATH`.

## Settings

| Setting | Effect |
|---|---|
| `sclang-lsp.server.path` | Absolute path to the binary. Overrides the search above. |
| `sclang-lsp.classLibraryPaths` | Index these directories *instead of* the platform defaults. Leave empty to let the server find the stock class library, `Extensions` and `downloaded-quarks`. |
| `sclang-lsp.trace.server` | Log LSP traffic. `messages` or `verbose` for debugging. |

Changing any of them restarts the server.

## What it does not do

No evaluation, no post window, no server control. Those need a live sclang, and
they are the editor's job rather than the language server's — running code is
not in the LSP's remit, and the existing `vscode-supercollider` routes it
through a custom `textDocument/evaluateSelection` method only because its server
already lived inside sclang.

**Do not enable this alongside `vscode-supercollider`.** Both contribute the
`supercollider` language, so you would get two servers answering and duplicate
completions and hovers.

## Licensing

The extension is GPL-3.0-or-later, matching the server.

`language-configuration.json` and `syntaxes/supercollider.tmLanguage.json` are
taken from [`vscode-supercollider`](https://github.com/scztt/vscode-supercollider),
MIT © 2022 Scott Carver. See [LICENSE-MIT](LICENSE-MIT).
