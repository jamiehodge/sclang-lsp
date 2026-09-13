# Architecture

The parts of the design that are decisions rather than code, recorded because
the code does not show them.

## The shape

```
editor  <--stdio/LSP-->  sclang-lsp
      \
       \--stdio-------->  sclang   (the editor's child, not the server's)
```

The server owns the conversation with the editor and the truth about document
text. It never spawns sclang, never talks to one, and does not care whether one
exists.

That inversion is the whole design. The existing `LanguageServer.quark` runs
the language server *inside* the sclang image, which is why it cannot report a
syntax error that stops the class library compiling, cannot survive a
recompile, and cannot offer anything while user code is evaluating.

### It also dissolves the stdio problem

`LanguageServer.quark` speaks LSP over **UDP**, because sclang writes to stdout
in many places and would corrupt a stdio stream. That is why Neovim, Helix and
Zed have never been supported — the issue has been open since 2024, and the
plan of record was to add stdio support to sclang itself.

None of that is needed. When the server is a separate process it owns stdio
with the editor, and an editor that also wants a live sclang holds that one
separately — on its own pipe, where sclang writing to stdout is ordinary output
rather than a corrupted protocol stream. No change to SuperCollider required.

## One tier

Parse every `.sc` file with this crate and index it. That is the whole
knowledge model: it works before sclang boots, after it crashes, on code that
does not compile, and on a machine with no SuperCollider installed at all.

This was once described here as tier 1 of two, with a tier 2 that supervised a
live image for `~envir` contents, SCDoc rendering and evaluation. That idea is
dropped. Everything needing a running image belongs to the editor, which is
already managing one for the user.

The cost of dropping it is small and specific, and it is listed under
*Deliberately not done* below rather than hidden.

### The test that keeps it honest

**Delete sclang, start the server, and confirm completion and goto-definition
still work on the class library.** `crates/sclang-lsp/tests/no_sclang.rs` runs
the shipped binary over real stdio with an empty `PATH`, so no sclang can be
found even if something later tries to look for one.

With one tier this is no longer a layering check but a standing guarantee: if
it ever fails, something has grown a dependency that the design says cannot
exist.

## Diagnostics

1. **Parser errors** — live, per keystroke. Produced by `sclang-syntax`;
   nothing else needed.
2. **Conservative semantic lints** over the index — unknown class, method not
   found, arity mismatch. Keep these narrow: fire only when the receiver is a
   literal class name. SuperCollider is dynamically typed and false positives
   here are worse than silence.
3. **Class-library compile errors** — not from this server at all. sclang
   prints them while compiling, before any image exists, which is why nothing
   that talks to a *running* sclang can report them: there is no sclang running
   yet to ask. Owning the child process is all it takes, and it does not have
   to be *this* process. The VS Code extension holds sclang's stdout and
   publishes them through `createDiagnosticCollection`, with no language server
   involved.

## Colour

Semantic tokens come from the buffer's own tree and **nothing else** — not the
symbol index, not a running sclang. That is the constraint that shapes the
feature.

Resolving class names against the index would let a known class look different
from an unknown one. It would also mean every open file changing colour part
way through startup, when the background scan lands, for information the
diagnostics already report properly. A colour that flickers is worse than a
colour that is merely coarse.

What the tree does settle is settled: `foo` in `x.foo` is a method, `foo` in
`foo(a)` is also a method — sclang reads it as `a.foo` — and a name declared as
`arg` is a parameter at every later use, through shadowing. None of that is
reachable from shape alone, which is what an editor's own grammar has.

The tokens are emitted for *every* lexeme, comments and literals included,
rather than only the semantic ones. A client with a grammar sets
`augmentsSyntaxTokens` and layers ours on top, so the overlap costs nothing;
a client without one gets highlighting it otherwise has no source for. That is
also why the capability is advertised unconditionally rather than per editor:
a client that does not consume semantic tokens never sends the request.

`editors/vscode` is now the second kind: the server is the only thing that
decides what colour anything is there. Its TextMate grammar went from 171 lines
to five rules, and the five that remain colour nothing anyone sees — semantic
tokens cover the same ranges and win.

They are kept because colour is not all a grammar does. VS Code reads the
TextMate token stream to decide whether a bracket is structure or text, and
semantic tokens cannot stand in: they are a colour layer applied after
tokenization, and no extension API supplies standard token types any other way.
Delete the string and comment rules and `"("`, `$(`, `'('` and `// (` all start
pairing with a later `)` — the same four cases that made counting parentheses
in the extension a non-starter.

So the division is: the parser says what a name *means*, and five regexes say
where the literals are. Nothing overlaps, and nothing in the grammar can be
wrong about an identifier, because it no longer has an opinion about one.

### Deltas

`full/delta` needs one piece of mutable state on the request path: the array
last sent for each open document, and the result id it went out under. Nothing
invalidates it on edit — diffing *against* the last response is the point — so
the only lifecycle event is `didClose`.

The diff is one edit: keep the longest common prefix and suffix, replace the
middle. A finer diff would not pay, because the relative encoding has already
localised the change. Every token's position is relative to the token before
it and the first on a line carries an absolute column, so renaming something on
line 100 leaves every token from line 101 on byte-identical. Rebuilding the
tokens was never the cost; re-sending several hundred kilobytes of JSON per
keystroke was.

## Documents

The server owns text: real incremental edits, version tracking, and the buffer
always beating the file on disk. Nothing else is consulted about what a buffer
contains. That deletes the entire `LSPDocument`/`Document` impedance mismatch
that the quark carries.

## Evaluation belongs to the editor

Running code is not in the language server's remit. LSP covers language
intelligence; execution is the editor's, or a debug adapter's. Every other
ecosystem arranges it that way — Calva owns the Clojure REPL, the Python
extension owns its terminals, and neither routes execution through a language
server.

The quark does route it through LSP, via a custom
`textDocument/evaluateSelection` method, but only because it was already living
inside sclang with a connection open. That is an accident of its architecture,
not a design.

So there is no sclang-side component here, and no plan for one. An editor that
wants evaluation, a post window, server control or `~envir` introspection
spawns sclang itself and keeps it entirely separate from this server. In
Neovim and Emacs, scnvim and scel already do exactly that, and offer no
language intelligence — the two halves fit together without overlapping.

`editors/vscode` is the worked example. It owns an sclang, writes to its stdin
and reads its stdout, and the server is not in that path. What crosses between
the halves is one request in the other direction: *what region is the cursor
in*, which is a question about syntax rather than about execution, and is
answered by the standard `textDocument/selectionRange`. Counting parentheses in
the extension would be the alternative, and it is wrong on `"("`, `$(`, `'('`
and `// (`.

The extension asks the server for that chain directly, over the language
client, rather than through `vscode.executeSelectionRangeProvider`. That
command merges every registered provider with VS Code's own, which ends every
chain with the whole buffer — so the editor would hand back a step the server
deliberately does not offer, and the answer would be "evaluate the file".

The chain has one rule that is easy to miss: the file's own range is not a
step. A file is a sequence of expressions rather than one, and a step covering
it cannot be told apart from a step covering a region — the parentheses at
either end of a buffer are not required to pair up, and in a file that opens
with `(` on line 1 and ends with `)` on the last, they do not. Where the text
really is a single expression the two coincide and the step stays.

This also removes a whole category of failure that the two-tier design had to
legislate against: nothing in the server can block on a live image, because
nothing in it talks to one.

### Talking to a sclang someone else started

Considered and rejected. `NetAddr.langPort` means a running sclang is listening
on UDP, but nothing in the class library interprets what arrives there, so it
would take sclang-side code in a quark or a `startup.scd` — the dependency this
design exists to avoid. And it buys only evaluation: without the pipe there is
no post window and no compile errors, the two things that come free with the
child. Owning the process is the cheaper half of that trade, not the expensive
one.


## sclang at development time

None of the above applies to the oracles. `oracle/` asks a running sclang what
classes and methods it compiled, and diffs that against what this crate
extracts from the same files; `lexer_oracle` diffs token streams against
upstream's `sc_lexer`; `tree_oracle` compares against a patched sclang's parse
dump.

Those are how the parser earns its claim to fidelity, and they run on a
developer's machine, never on a user's. "No dependency on sclang" is a
statement about the shipped binary.

## Deliberately not done

- **No talking to sclang.** Not spawned, not connected to, not required. See
  above.
- **No `~envir` completion.** Environment variables live in a running image and
  nothing static can enumerate them. This is the one real capability given up
  by dropping the live tier, and it is worth the exchange.
- **No type inference.** Without types, `implementors()` — every class defining
  a selector — is the honest answer for completion after a `.`. Inlay hints and
  keyword-argument completion go further and stay silent unless the receiver is
  a literal class name, because both render as though they were in the source
  and a guess there would read as a fact.
- **No renaming methods.** Dispatch is dynamic, so nothing distinguishes one
  class's `play` from another's. Rename runs only where the occurrence set is
  provably complete: function locals and class names.
- **No linking `sc_lexer`.** Upstream's lexer is used as a *test oracle*, not a
  dependency, so the crate stays pure Rust and cross-compiles without a C++
  toolchain. Their C++ API is newer and moves faster than the language does.
- **No tree-sitter.** A language server wants a parser it controls, and the one
  here already knows more than a grammar can: which lowercase word is a
  selector, which is a parameter, and which is a local. That knowledge reaches
  the editor as semantic tokens instead — see *Colour* below. Tree-sitter is
  still the right answer for a client that consumes no semantic tokens, and
  that grammar work is worth upstreaming, but it is no longer the only way an
  editor can colour SuperCollider correctly.

## Open threads

- [supercollider#7709](https://github.com/supercollider/supercollider/pull/7709)
  — the five `DumpParseNode.cpp` fixes, upstream. The tree oracle needs a
  patched sclang until that lands.
- Two class-library files are not valid UTF-8 (Latin-1 from the 2000s). They
  are the only known correctness gap in the symbol layer, and decoding them
  needs a decision about what byte offsets mean afterwards.
- `langutils/` upstream holds only `sc_lexer` today. The directory shape and
  the new `%locations` support in the bison grammar both suggest a parser
  extraction may follow; worth watching rather than pre-empting.
