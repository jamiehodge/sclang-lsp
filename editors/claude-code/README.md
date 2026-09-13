# SuperCollider for Claude Code

Claude Code speaks LSP natively, so this plugin is only a declaration: it points
Claude at `sclang-lsp` and tells it which file extensions to use it for. There
is no adapter, no MCP server and no code here.

## Install

```
/plugin marketplace add jamiehodge/sclang-lsp
/plugin install sclang-lsp
```

The server binary has to be on your `PATH`. Download one from the
[releases page](https://github.com/jamiehodge/sclang-lsp/releases/latest) and
put it somewhere on `PATH`, or build it:

```bash
cargo build --release   # then copy target/release/sclang-lsp onto your PATH
```

SuperCollider itself is not required. The server reads the class library by
parsing it, so it works with sclang absent, broken or busy — including on a
class library that does not compile.

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
narrow at all, on purpose.

## What it does not do

Completion, signature help, inlay hints and diagnostics are all implemented by
the server, but they are driven by an editor rather than requested by an agent,
so Claude does not use them. Nothing here runs SuperCollider code either — see
the [VS Code extension](../vscode) for that.

## License

GPL-3.0-or-later, matching the server.
