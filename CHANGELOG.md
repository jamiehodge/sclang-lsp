# Changelog

Notable changes, newest first. Versions follow [semver](https://semver.org),
with the usual pre-1.0 caveat that the minor number carries breaking changes.

## [0.8.0] — 2026-09-13

### Added

- **Document highlight.** The other places a name is written in the file,
  with the declaration marked as a write and the uses as reads. The same
  question find-references answers and the same three standards of proof —
  exact for a local, exact for a class name, textual for a selector — but a
  highlight is a tint on a word already on screen, so the textual answer is
  worth giving here even though it is too weak to rename on.

- **Files changing on disk reach the index.** It previously heard only about
  buffers the editor had open, so a `git checkout`, a quark install, or an edit
  made in another program left goto-definition pointing at locations that had
  moved — with nothing to say so. The server now registers a `**/*.sc` watch
  with clients that support one and re-reads what it is told about.

  An open buffer still outranks the file. The server owns document text, so
  what is on disk beneath an unsaved edit is the stale copy, and `didSave`
  already covers the saved case.

## [0.7.3] — 2026-09-13

### Fixed

- **Evaluating a region really does evaluate just that region.** 0.7.2 stopped
  the *server* offering a selection-range step for the whole file, which was
  necessary and not sufficient: the extension was asking
  `vscode.executeSelectionRangeProvider`, and that command merges every
  registered provider with VS Code's own `WordSelectionRangeProvider`, whose
  last contribution is unconditionally `getFullModelRange()`. The editor put
  the step straight back, and ⌘⏎ went on running the whole file.

  The extension now sends `textDocument/selectionRange` to the server through
  the language client, which is a direct request with nothing merged into it.
  It is also the question actually meant: what the parser saw, rather than what
  every provider in the editor thinks.

  One behaviour change falls out of it. Blocks were always the server's
  contribution, but the old call had the editor's own providers to fall back
  on; now, if the server is not up yet, there is no chain and ⌘⏎ evaluates the
  current line. That fallback was what produced the wrong answer.

  `scripts/e2e-selection.js` covers both shapes that were wrong, and is now the
  same path the extension takes: the built server over stdio, through
  `pickBlock`. Reverting the 0.7.2 half fails it, which is the check that says
  both halves are load-bearing.

## [0.7.2] — 2026-09-13

### Fixed

- **A file with no trailing newline evaluated whole instead of block by
  block.** The selection-range chain offered a step for the file itself, and a
  file that happens to begin with `(` and end with `)` — two regions stacked
  up, with no newline after the last one — is indistinguishable at that step
  from one enormous region. The extension takes the outermost `( … )` in the
  chain, so ⌘⏎ anywhere in such a file ran every region at once: the SynthDef,
  the Synth, and every pattern.

  The parentheses were never required to pair up. In a file starting `(` on
  line 1 and ending `)` on the last line, the opening one is closed long
  before.

  The file's range is no longer a step, because a file is a sequence of
  expressions rather than one. When the text really is a single expression the
  two coincide and the step stays, so a buffer holding exactly one `( … )`
  still evaluates, and expand-selection still reaches all of a call that fills
  the file. That second case is why the fix is here rather than in the
  extension: from the chain alone those ranges are identical.

## [0.7.1] — 2026-09-13

### Fixed

- **Brackets inside strings and comments are text again.** Removing the
  TextMate grammar in 0.7.0 took more with it than colour: VS Code decides
  whether a `(` is structure from the TextMate token stream, and semantic
  tokens cannot stand in — they are a colour layer applied after tokenization,
  and no extension API supplies standard token types any other way. So `"("`,
  `$(`, `'('` and `// (` all began pairing with a later `)` in bracket
  matching, bracket-pair colouring and auto-closing.

  The grammar is back at five rules: strings, symbols, char literals and
  comments, nesting included. None of them guesses what an identifier means,
  which is where the 171-line version went wrong, and none of them changes what
  anything looks like — semantic tokens cover the same ranges and win wherever
  both apply.

  The char literal is scoped `string.other.character` rather than
  `constant.character`, which reads oddly until you know that VS Code derives a
  token's type by matching `\b(comment|string|regex|regexp)\b` against the
  scope name. Only a `string` or `comment` scope suppresses the brackets inside
  it, and `$(` is one of the four cases. `src/test/grammar.test.ts` pins all of
  it against the tokenizer VS Code itself uses.

## [0.7.0] — 2026-09-13

### Added

- **Semantic tokens**: `full`, `range` and `full/delta`. Colour now comes from
  the parse tree
  rather than only from the editor's own grammar, which has to guess from shape
  alone: a lowercase word is a variable, a capitalised one is a class. The tree
  knows better. `blend` in `x.blend(1)` is a method; `foo` in `foo(a)` is also
  a method, because sclang reads it as `a.foo`; a name declared as `arg` stays
  a parameter at every later use, through shadowing; and `~out` is one token
  including its tilde.

  Nothing consults the symbol index, so colour does not change when the class
  library scan lands — a flicker would cost more than the extra precision is
  worth. Every lexeme is emitted, comments and literals included, so a client
  with no SuperCollider grammar gets highlighting it otherwise has no source
  for; one with a grammar layers these over it.

  The delta encoding fails silently — a wrong offset raises nothing anywhere,
  it just slides colour down the file — so it is checked as a property rather
  than by example: every token in order, on one line, and landing on the source
  it claims. In CI that runs over committed inputs; `examples/token_sweep.rs`
  runs the same checks over a real class library and the help-file corpus,
  where it clears 5,465 files and 546,358 tokens.

  A delta keeps one thing on the request path — the array last sent for each
  open document, and the id it went out under — and pays for it in transfer.
  The diff is a single edit, which is enough because the relative encoding has
  already localised the change: renaming something on line 100 leaves every
  token from line 101 on byte-identical. A rename on line 100 of a 200-line
  file sends 10 integers where the full array is 3,000.

### Changed

- **The VS Code extension has no TextMate grammar.** Colour comes from the
  server alone. The grammar could only guess from shape — every lowercase word
  a variable, every capitalised one a class — and the semantic tokens were
  already overriding it nearly everywhere.

  Colour now waits on the server, which in practice is not a wait: spawn to
  first tokens is 4–6ms, including a thousand-line class file, because
  `initialize` returns before the class library is read and the tokens never
  consult the index. What is genuinely given up is the failure case — if the
  server does not start, nothing paints the file at all, where the grammar used
  to.

  The other consequence is real. VS Code takes its bracket matching and
  bracket-pair colouring from TextMate tokens, which is how it knows a `(`
  inside a string or comment is not structure; semantic tokens do not feed
  either, so `"("` may now pair with a later `)`. Evaluation is unaffected by
  construction: <kbd>⌘⏎</kbd> asks the server for the enclosing block through
  `textDocument/selectionRange`, which reads the parse tree and has never
  counted parentheses.

## [0.6.2] — 2026-09-13

### Fixed

- **sclang is no longer left running when the extension goes away.**
  `deactivate` awaited the language client and not sclang, and `dispose`
  returns void, so nothing waited for the child — the extension host could exit
  first. It now waits for both, with a synchronous kill registered on
  `process.exit` for when it cannot.

  This matters more than a stray process usually would: an sclang whose stdin
  has gone spins at 100% of a core, and because scsynth competes with it for
  CPU, the first symptom is dropouts in what is playing rather than anything
  about the editor.

## [0.6.1] — 2026-09-13

### Fixed

- **An adverb may be negative.** `adverb : '.' integer` and `integer` is
  `INTEGER | '-' INTEGER`, so `z +.-1 y` shifts the other way from `z +.1 y`.
  The adverb rule accepted only an unsigned one.

- **Non-breaking spaces are spaces.** Treating every non-ASCII character as
  part of a name joined a stray `\u{a0}` to the token after it, turning a list
  into a syntax error. sclang splits them, and says which is which: `x = [1,
  \u{a0}2]` compiles, so a non-breaking space separates; `var ±x = 1;` compiles
  and `1 ± 2` does not, so `±` is part of a name.

  With these two, the script oracle reports **0** disagreements: every one of
  the 4,785 snippets sclang accepts now parses here.

## [0.6.0] — 2026-09-13

### Fixed

Six divergences from `lang11d`, all in script syntax, all found by the new
oracle and none visible to any other.

- **Literal collections at the start of a statement.** `Set[1, 2, 3]` begins
  exactly like `Array[slot] : ArrayedCollection { … }`, and was read as one.
  82 of the 125 first-run disagreements.
- **`.sc` and `.scd` are read differently**, as `root : classes | … | INTERPRET
  cmdlinecode` says they should be. `Routine { … }` is a class definition in a
  class file and a call in a script, and nothing in the text says which.
- **List comprehensions** — `{: [a, b], a <- (0..3), (a+b).isPrime }`, and the
  `{; … }` form, with all six `qual` shapes.
- **`(:2..5)`**, the series form that yields a Routine.
- **`;` inside an argument**, which `exprseq` allows: `max(b = a * 2; b + 5, 10)`
  is a two-argument call.
- **`key: value` in array literals** where the key is any expression, as in
  `#[freq, sustain]: Ptuple(…)`.
- **Non-ASCII identifiers.** The lexer rejected them on the stated grounds that
  `PyrLexer.cpp` is ASCII-only. sclang disagrees: `±` alone compiles, `var ±x`
  compiles, `1 ± 2` does not — which is an identifier character, not an
  operator. Fixing it also fixed a panic on the first non-ASCII byte.

### Added

- **A script oracle.** `./oracle/run-scd.sh` compiles 4,785 snippets — every
  `code::` block in the help files, plus installed `.scd` files — with
  `String:compile`, which parses without running, and diffs the verdicts.
  Disagreements went 125 -> 4.

## [0.5.0] — 2026-09-13

### Fixed

- **`var` and `arg` declarations work in a top-level block.** `( var a = 1; … )`
  is how most of a `.scd` file is written, and `cmdlinecode` in `lang11d` spells
  it out — `'(' argdecls1 funcvardecls1 funcbody ')'`, along with the same
  declarations bare at the top of a script. Neither was implemented, so both
  reported syntax errors on correct code.

  The declarations are also a scope now. A `var` in the block you are working in
  is offered by completion and resolved by hover and goto, as it already was
  inside a function body.

- **Adjacent top-level blocks are separate again.** A `.scd` file is normally a
  sequence of `( … )` blocks with no separators between them, evaluated one at
  a time — the file is never parsed as a unit. The parser allowed *any*
  expression to be followed by an argument list, so a `)` on one line and a `(`
  on the next read as a call, merging two blocks into one. Evaluating either
  sent both, and sclang answered `unexpected '(', expecting end of file`.

  `lang11d` has no `expr '(' arglist ')'` production: a callee is a `name` or a
  `classname`, and the one exception, `'(' binop2 ')' '(' … ')'`, this parser
  does not reach anyway. Calls are now restricted to match, with trailing `{ }`
  blocks still attaching to a completed call so `if (a) { } { }` is unchanged.

  The corpus improves with it — 622 of 628 files parsed clean before, 625 after
  — and the symbol and resolution oracles are unmoved.

### Added

- **`textDocument/implementation`.** Goto-definition has to pick one place;
  this lists them all, which in a dynamically dispatched language is usually
  the question with a real answer. On a selector, every class defining it. On a
  method definition, every sibling of the override. On a class name, its
  subclasses.

- **A Claude Code plugin**, in [`editors/claude-code`](editors/claude-code).
  Claude Code speaks LSP natively, so the plugin is a declaration and nothing
  else — no adapter, no MCP server, no code. The repository is its own
  marketplace: `/plugin marketplace add jamiehodge/sclang-lsp`.

  Two things that are easy to get wrong and report nothing: `lspServers` goes in
  `plugin.json` rather than the marketplace entry the official directory uses,
  and the binary must be on the `PATH` the *app* inherits, which on macOS
  excludes `~/.cargo/bin`.

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

[0.8.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.8.0
[0.7.3]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.7.3
[0.7.2]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.7.2
[0.7.1]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.7.1
[0.7.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.7.0
[0.6.2]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.6.2
[0.6.1]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.6.1
[0.6.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.6.0
[0.5.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.5.0
[0.4.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.4.0
[0.3.1]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.3.1
[0.3.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.3.0
[0.2.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.2.0
[0.1.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.1.0
