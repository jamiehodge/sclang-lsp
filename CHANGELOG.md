# Changelog

Notable changes, newest first. Versions follow [semver](https://semver.org),
with the usual pre-1.0 caveat that the minor number carries breaking changes.

## [0.2.0] — 2026-09-13

### Added

- **`textDocument/selectionRange`.** The chain of progressively larger
  syntactic regions around a position: expand-selection in an editor, and the
  only sound way to find the block around a cursor. Counting parentheses is
  wrong on `"("`, `$(`, `'('` and `// (`; a lossless tree is not.
- **The VS Code extension runs SuperCollider code**, in a child process it owns
  rather than through the server. <kbd>⌘⏎</kbd> evaluates the selection, the
  enclosing block or the current line, output goes to a post window, and
  **class-library compile errors appear in the Problems panel** — those are
  printed before any image exists, so only something holding sclang's output
  can report them at all.
- `sclang-lsp.eval` takes the code to run as its argument, so any SuperCollider
  expression can be bound to a key without the extension shipping a command for
  it. <kbd>⌘.</kbd> is bound to the hard stop by default.

### Changed

- The READMEs are written for people wanting to use this rather than work on
  it. The derivation and the differential oracles moved to
  [CONFORMANCE.md](CONFORMANCE.md), and the extension's build loop to
  `editors/vscode/DEVELOPING.md`; nothing was dropped.
- "It does not run anything" is now stated about the **server**, which is where
  the guarantee always lived and where `tests/no_sclang.rs` enforces it.

### Fixed

- The workspace is clippy-clean at `-D warnings`, and CI holds it there.

## [0.1.0] — 2026-09-12

First release.

- Lexer ported from `PyrLexer.cpp`, parser derived from the bison grammar in
  `Bison/lang11d`, and a lossless error-tolerant tree over both.
- Symbol index over the class library, `Extensions` and `downloaded-quarks`.
- Diagnostics, completion, signature help, inlay hints, hover,
  goto-definition, references, rename, and document and workspace symbols.
- Verified against sclang itself: token-for-token against upstream's
  `sc_lexer`, symbol-for-symbol against the compiled class library, and
  selector-for-selector against a patched sclang's parse dump.

[0.2.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.2.0
[0.1.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.1.0
