# Architecture

The parts of the design that are decisions rather than code, recorded because
the code does not show them.

## The shape

```
editor  <--stdio/LSP-->  sclang-lsp  <--UDP/OSC-->  sclang (supervised, optional)
```

The server owns the conversation with the editor and the truth about document
text. sclang is something it *asks*, and can run without.

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
with the editor, and sclang keeps UDP, where it is already comfortable. No
change to SuperCollider required.

## Two tiers, and one rule

**Tier 1 — static, always available.** Parse every `.sc` file with this crate
and index it. Works before sclang boots, after it crashes, and on code that
does not compile. This is the floor, not a fallback.

**Tier 2 — the live image, best effort.** Only what an image can answer:
`~envir` contents, SCDoc rendering, runtime-resolved dispatch, server state,
evaluation.

**The rule: no LSP request may block on tier 2.** Async with a timeout,
degrade to tier 1. That single constraint is what keeps completion instant
while sclang is booting, recompiling, wedged in a user's `while` loop, or dead.

### The test that keeps it honest

**Delete sclang, start the server, and confirm completion and
goto-definition still work on the class library.** If that fails, the layering
has drifted and tier 2 has become load-bearing. Worth writing early.

Reflection data may be *cached* to disk and reused — that is a build artefact,
not a runtime dependency. But it must never be the only source: merge with
static as the base, and never let a missing enrichment remove a symbol.

## Diagnostics, from three sources

1. **Parser errors** — live, per keystroke. Already produced by
   `sclang-syntax`; nothing else needed.
2. **Supervised sclang stdout** — class-library compile errors, which sclang
   prints as `in <file> line <n> char <m>` (PyrLexer.cpp). Nobody can capture
   these today. Owning the child process is all it takes, and it is the most
   requested missing feature.
3. **Conservative semantic lints** over the index — unknown class, method not
   found, arity mismatch. Keep these narrow: fire only when the receiver is a
   literal class name. SuperCollider is dynamically typed and false positives
   here are worse than silence.

## Documents

The server owns text: a rope, real incremental edits, version tracking. sclang
never sees a document — it sees a source string and a path when asked to
evaluate something. That deletes the entire `LSPDocument`/`Document`
impedance mismatch that the quark carries.

## The sclang side, if it is ever needed

Kept as small as it can be defended, and **not a quark**.

A quark is user-installed, so it version-skews against the server and produces
bug reports nobody can reproduce. Instead: the server already spawns sclang
with `-l <config>` (as the VSCode extension does), so it can generate that
config with a private `includePaths` entry pointing at a directory it
materialises from bytes compiled into the binary. The user installs nothing and
skew is structurally impossible.

Rules for that sidecar: an OSC responder, evaluation, environment
introspection. **No `SystemOverwrites`, no document interception, no boot-order
changes.** The quark's `extMain.sc` replaces `Main.startup` and skips
`StartUp.run`, which is why code behaves differently under it — that must not
recur. If the sidecar seems to need `SystemOverwrites`, something belongs in
the server instead.

Evaluation goes on its own channel, never the analysis path, so a long-running
expression cannot freeze completion.

## Deliberately not done

- **No type inference.** Without types, `implementors()` — every class defining
  a selector — is the honest answer for completion after a `.`.
- **No linking `sc_lexer`.** Upstream's lexer is used as a *test oracle*, not a
  dependency, so the crate stays pure Rust and cross-compiles without a C++
  toolchain. Their C++ API is newer and moves faster than the language does.
- **No tree-sitter.** It remains the right tool for editor highlighting, and
  the grammar work done separately is worth upstreaming, but a language server
  wants a parser it controls.

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
