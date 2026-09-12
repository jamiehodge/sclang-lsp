# SuperCollider for VS Code, via `sclang-lsp`

Language features served by [`sclang-lsp`](../../README.md): diagnostics as you
type, completion, signature help, inlay hints, hover, goto-definition,
references, rename, and document and workspace symbols.

The server parses SuperCollider itself, so all of it works with sclang absent,
broken, or busy — including on a class library that does not compile.

**It does not run SuperCollider code.** No evaluation, no post window, no
server control. Those need a live sclang and belong to the editor rather than
to a language server, so this extension is for reading and navigating code, not
for playing it.

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

[`sample.scd`](sample.scd) is a guided tour of every feature, in order, with a
deliberate syntax error at the end for the diagnostics.

## Installing it

To use it in your normal editor rather than a development window:

```bash
cargo build --release          # from the repository root
npm install && npm run package # bundles the binary into the VSIX
code --install-extension sclang-lsp-0.1.0.vsix
```

`npm run package` copies `target/release/sclang-lsp` into `server/` first. An
installed extension lives in `~/.vscode/extensions` with no repository near it,
so without the bundled copy there is nothing for it to find and you would have
to set `sclang-lsp.server.path` by hand. Bundling makes the VSIX specific to
the platform it was built on, which is what the per-target release artifacts
exist for.

> **Disable any other SuperCollider extension first.** Two extensions
> contributing the `supercollider` language means two servers answering every
> request, and duplicate completions and hovers.

## Settings

| Setting | Effect |
|---|---|
| `sclang-lsp.server.path` | Absolute path to the binary. Overrides the search above. |
| `sclang-lsp.classLibraryPaths` | Index these directories *instead of* the platform defaults. Leave empty to let the server find the stock class library, `Extensions` and `downloaded-quarks`. |
| `sclang-lsp.trace.server` | Log LSP traffic. `messages` or `verbose` for debugging. |

Changing any of them restarts the server.

## Two editor behaviours worth knowing

**Signature help is not shown just because the cursor is inside a call.** VS
Code requests it when you type `(` or `,`, or when you ask for it with ⌘⇧Space
(*Trigger Parameter Hints*) — not when the cursor arrives in parens that are
already closed. The server answers at every position inside a call; the editor
simply does not ask on cursor movement.

**Inlay hints follow `editor.inlayHints.enabled`**, which some setups turn off
or set to `onUnlessPressed`. They need no cursor and appear on their own.

## What it does not do

No evaluation, no post window, no server control. Running code is not in the
LSP's remit — every other ecosystem puts execution in the editor. The quark
routes it through a custom `textDocument/evaluateSelection` method only because
its server already lived inside sclang with a connection open, which is an
accident of that architecture rather than a design to copy.

**Do not enable this alongside another SuperCollider extension.** Two
extensions contributing the `supercollider` language means two servers
answering, and duplicate completions and hovers.

## Licensing

The extension is GPL-3.0-or-later, matching the server.

`language-configuration.json` and `syntaxes/supercollider.tmLanguage.json` are
taken from [`vscode-supercollider`](https://github.com/scztt/vscode-supercollider),
MIT © 2022 Scott Carver. See [LICENSE-MIT](LICENSE-MIT).
