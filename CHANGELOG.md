# Changelog

Notable changes, newest first. Versions follow [semver](https://semver.org),
with the usual pre-1.0 caveat that the minor number carries breaking changes.

## [0.4.0] — 2026-09-13

### Fixed

- **A class written without `: Super` now inherits `Object`.** SuperCollider
  resolves it that way; the index had recorded it as having no superclass at
  all, which stopped every superclass walk there. 215 classes in the stock
  library are written like that, `AbstractFunction` among them — so the chain
  from any UGen or any Pattern never reached `Object`, and completion on such a
  receiver had never offered `postln`, `dump` or anything else Object defines.

### Added

- **Receivers narrow when the class is knowable.** `Pbind(...).play` resolves to
  `Pattern:play` rather than to every `play` in the image, and `"x".reverse`,
  `[1, 2].sum`, `{ }.value` and the other literal forms resolve on their own
  class. Completion, hover, goto-definition and find-references use it.

  Inlay hints and keyword-argument completion deliberately do not: those render
  as though they were in the source, and `Foo(...)` being an instance of `Foo`
  is convention rather than guarantee.

- **A resolution oracle.** `./oracle/run-resolution.sh` asks sclang what its own
  dispatch would select — via `findRespondingMethodFor` — for every class
  against every selector in its superclass chain, and diffs 750,000 of those
  against the server. It found the superclass bug above on its first run.

## [0.3.1] — 2026-09-13

### Fixed

- **An idle server no longer burns a CPU core.** The event loop selected over
  the channel carrying the class library scan, and once that scan arrived its
  sender was dropped — leaving a *disconnected* channel in the select. A
  disconnected channel is always ready, so the loop spun at 100% CPU for the
  life of the process, from the moment indexing finished. Every request still
  answered correctly, which is why nothing noticed. Present since 0.1.0.

- **Unsaved buffers work.** The client attached only to the `file` scheme, so a
  buffer that had never been saved — `untitled`, and where SuperCollider tends
  to actually get written — got no diagnostics, no completion and no hover at
  all. Underneath that, a synthetic path could not be turned back into a URI,
  so goto-definition and find-references would have skipped unsaved buffers
  even once the client attached.

## [0.3.0] — 2026-09-13

The server and the VS Code extension share a version number from here on. A
release builds both together, so one number answers "which server is inside
this extension?". The extension jumps 0.2.3 -> 0.3.0 to meet the server;
nothing about its behaviour changed in that step.

### Added

- **The extension is downloadable.** Each of the five server targets now also
  produces a VS Code platform-specific build, so the release page carries a
  `.vsix` for `darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64` and
  `win32-x64`. Previously the only way to get the extension was to build it.
- An icon, and the Marketplace metadata that goes with it.
- [CONTRIBUTING.md](CONTRIBUTING.md), issue forms and a pull request template.
- `repository`, `keywords`, `categories` and `rust-version` on the crates. The
  MSRV is 1.98, which is the toolchain CI already pins clippy to, so that job
  doubles as the check that it holds.
- This changelog, and CI/release/licence badges on the README.

### Fixed

- The workspace builds without warnings. Three had accumulated, which is why
  clippy was strict only on the server; it is now `-D warnings` everywhere,
  examples included.

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

[0.4.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.4.0
[0.3.1]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.3.1
[0.3.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.3.0
[0.2.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.2.0
[0.1.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.1.0
