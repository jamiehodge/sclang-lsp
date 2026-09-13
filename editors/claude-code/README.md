# SuperCollider for Claude Code

Claude Code speaks LSP natively, so this plugin is only a declaration: it points
Claude at `sclang-lsp` and tells it which file extensions to use it for. There
is no adapter, no MCP server and no code here.

## Install

```
/plugin marketplace add jamiehodge/sclang-lsp
/plugin install sclang-lsp
```

The server binary has to be on the `PATH` **Claude Code itself sees**, which is
not always the one your shell has. On macOS an app launched from the Finder or
Dock inherits a minimal environment: `~/.cargo/bin` is not in it, so
`cargo install sclang-lsp` leaves a binary that `which` finds and Claude
cannot. The symptom is `ENOENT: sclang-lsp` from a server that registered fine.

Somewhere already on the app's path is the safe answer:

```bash
# macOS, Apple Silicon — /opt/homebrew/bin is user-writable
cargo install --path crates/sclang-lsp
ln -sf ~/.cargo/bin/sclang-lsp /opt/homebrew/bin/sclang-lsp
```

Or download a binary from the
[releases page](https://github.com/jamiehodge/sclang-lsp/releases/latest) and
put it in `/usr/local/bin` (`sudo` on macOS) or anywhere else the app can see.

SuperCollider itself is not required. The server reads the class library by
parsing it, so it works with sclang absent, broken or busy — including on a
class library that does not compile.

## Where the declaration goes

`lspServers` belongs in `.claude-plugin/plugin.json`. The official marketplace
carries it in the *marketplace* entry instead, and copying that shape looks
right, validates, installs — and never registers a server, because Claude reads
it from the plugin manifest. A submission to the official directory would want
it in both places.

## What Claude can do with it

| | |
|---|---|
| `hover` | Signature, superclass chain, and the comment above the definition |
| `goToDefinition` | Exact for a class, a class receiver or a local; every implementor otherwise |
| `goToImplementation` | Every class defining a selector; on a class name, its subclasses |
| `findReferences` | Exact for a local or a class name; textual for a selector, since dispatch is dynamic |
| `documentSymbol` | Classes with their methods nested |
| `workspaceSymbol` | Classes and methods across the class library and your own code |

`goToImplementation` is the one worth knowing about. SuperCollider dispatches at
run time, so "where is this defined" usually has many answers — and that list is
the honest one. Goto-definition narrows where it can; implementation does not
narrow at all, on purpose. On `play` in the stock class library it returns 32
places, which is the point rather than a failure.

Diagnostics arrive too, unasked: a syntax error in a file Claude reads is
reported as it would be in an editor.

## What it does not do

Completion, signature help, inlay hints and diagnostics are all implemented by
the server, but they are driven by an editor rather than requested by an agent,
so Claude does not use them. Nothing here runs SuperCollider code either — see
the [VS Code extension](../vscode) for that.

## License

GPL-3.0-or-later, matching the server.
