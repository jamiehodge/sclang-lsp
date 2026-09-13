# Contributing

Bug reports, corrections to the editor setup snippets, and patches are all
welcome. This file is the practical part; [ARCHITECTURE.md](ARCHITECTURE.md)
explains the decisions behind the shape of the project, and is worth reading
before proposing anything structural.

## Getting set up

```bash
git clone https://github.com/jamiehodge/sclang-lsp
cd sclang-lsp
cargo test
```

No SuperCollider install is needed. The test suite is hermetic: every test that
needs a class library builds one in a temp directory, so the platform defaults
are never consulted.

For the VS Code extension, see
[`editors/vscode/DEVELOPING.md`](editors/vscode/DEVELOPING.md).

## Before you open a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test
```

CI runs exactly these, plus the extension's own `npm test`. Clippy is pinned to
the Rust version in `rust-version`, not to `stable`, so that a new lint in a
new toolchain does not turn an unrelated pull request red.

## Where things live

| | |
|---|---|
| `crates/sclang-syntax` | Lexer, parser, and the lossless tree |
| `crates/sclang-index` | Classes, methods and their locations |
| `crates/sclang-lsp` | The server, and one module per LSP request |
| `editors/vscode` | The extension |
| `oracle/` | Differential checks against a real sclang |

## Changing the lexer or parser

These are derived from sclang's own front end — `PyrLexer.cpp` and the bison
grammar in `Bison/lang11d` — and the derivation is the reason the project can
claim to be right about the language. So:

- **Match sclang's behaviour, not what seems reasonable.** SuperCollider has
  surprising rules. `1 + 2 * 3` is 9, because binary operators are left to
  right with no precedence. If you find yourself adding a rule that is not in
  `lang11d`, that is the signal to stop.
- **Deliberate departures are marked at the site.** There are three, all
  following from the fact that an editor mostly sees invalid programs: the
  token stream is lossless, nothing aborts on bad input, and every token
  carries byte ranges. New ones need the same treatment.
- **Run the oracles** if the change could affect what is parsed.
  [CONFORMANCE.md](CONFORMANCE.md) explains each one. They need a real
  SuperCollider install and are not part of CI, so they are the maintainer's
  responsibility in practice — but flagging in the pull request that a change
  might move those numbers is helpful.

## Adding an LSP request

One module per request under `crates/sclang-lsp/src/features/`, a match arm in
`server.rs`, and the capability in `capabilities()`. Tests go in
`tests/protocol.rs` and speak the protocol over a real transport rather than
calling the feature function, which is what catches parameter shapes and
capability negotiation.

The standing rule is in ARCHITECTURE.md and enforced by
`tests/no_sclang.rs`: the server never spawns sclang, contacts one, or needs
one to exist. Anything requiring a live image belongs in the editor.

## Where the bar is high

**Silence beats a guess.** SuperCollider is dynamically typed, and several
features deliberately say nothing rather than say something plausible. Inlay
hints and keyword-argument completion appear only when the receiver is a
literal class name, because both render as though they were in the source.
Rename refuses methods, because dispatch is dynamic and the occurrence set
cannot be enumerated. A patch that relaxes one of these needs to argue why the
wrong answer is cheap.

## Reporting a bug

The issue templates ask for the editor, the OS and the server log. For anything
about completion or diagnostics, the smallest `.sc` or `.scd` file that shows
it is worth more than a description.
