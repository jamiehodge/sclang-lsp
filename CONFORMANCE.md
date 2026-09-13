# Conformance

How this crate's front end relates to sclang's own, and how that relationship
is checked. The short version is in the [README](README.md); this is the
evidence behind it.

## Why derive rather than reimplement

SuperCollider's compiler front end is the only authority on what the language
actually is. A hand-written grammar maintained separately will drift, and the
drift is invisible — it produces trees that parse without error but are wrong.
(The tree-sitter grammar, for instance, modelled SuperCollider as having C-like
operator precedence for years. It doesn't: `1 + 2 * 3` is 9, not 7.)

So both halves of this crate come from `lang/LangSource` in the SuperCollider
source tree:

| This crate | Derived from |
|---|---|
| `lexer.rs` | `PyrLexer.cpp` — the hand-written scanner |
| `parser.rs`, `grammar.rs` | `Bison/lang11d` — the bison grammar |

The grammar extraction is mechanical: strip the semantic actions from
`lang11d` (they are 80% of the file and entirely sclang-internal) and 325 lines
of pure grammar remain, which bison accepts with zero conflicts.

## Where it deliberately differs

sclang's front end exists to compile valid programs. A language server mostly
sees *invalid* ones, because the user is still typing. Three departures follow
from that, and each is marked at the site in the source:

1. **Lossless.** Whitespace and comments are tokens, not skipped. The token
   stream tiles the input exactly, so it round-trips to the byte. This is what
   makes formatting and refactoring possible later.
2. **Error tolerant.** Nothing aborts. Unrecognised input becomes an `Error`
   token and lexing continues. sclang's parser has no error productions at all
   — it stops at the first syntax error — which is precisely why it cannot be
   used directly for editor tooling.
3. **Ranges.** Every token carries start and end byte offsets. `PyrParseNode`
   carries only a start line and column.

No runtime dependency on sclang. The derivation happens at development time;
the resulting server is a standalone binary.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the runtime design: the process
topology, why the server owns stdio, and why running code belongs to the editor
rather than to a language server.

## The corpus sweep

The lexer is checked against real SuperCollider source in bulk — the class
library, plus installed Extensions and quarks:

```
files              : 682
tokens             : 805,030
error tokens       : 0 (0.0000%)
lossless (tokens)  : ALL FILES
---- parser ----
files parsed clean : 670 / 680 (98.53%)
not valid UTF-8    : 2 (excluded)
classes found      : 2,075
methods found      : 12,263
lossless (tree)    : ALL FILES
```

**Every `.sc` class file in the corpus parses.** The 10 remaining failures are
all `.scd` scripts, and 6 of those have genuinely unbalanced delimiters — files
sclang rejects too. The rest are example scripts meant to be evaluated block by
block rather than parsed as a unit.

The tree is lossless on *every* file, failures included: error recovery keeps
the rest of a broken file intact, which is the property that matters in an
editor.

Reproduce with:

```bash
cargo run --release --example conformance -- \
  /Applications/SuperCollider.app/Contents/Resources/SCClassLibrary \
  ~/Library/Application\ Support/SuperCollider/Extensions
```

Note that lexing is a substantially lower bar than parsing; a clean sweep here
means the token model is faithful, not that the language is fully handled.

## Differential oracles

Two, because they check different layers.

### Lexer: against `sc_lexer`

Upstream extracted their lexer into `langutils/sc_lexer`, a standalone library
with a token dumper. We do **not** link it — this crate stays pure Rust so it
cross-compiles without a C++ toolchain, and because their C++ API is newer and
moves faster than the language does. Instead we build it and diff token streams,
which buys fidelity without the build dependency.

```bash
./oracle/build-sc-lexer.sh
cargo run --release --example lexer_oracle -- oracle/sc_lexer_dump <dir>...
```

```
files compared : 680
agreed exactly : 680 / 680 (100.00%)
```

Token for token, including trivia. The two taxonomies are reconciled on one
axis only: upstream splits whitespace by character class (`Space`/`NewLine`/
`Tab`) where we emit one token per run, so runs are coalesced before comparing.

### Symbols: against the compiled class library

This checks the real `sclang-index`, not a throwaway copy — so the index ships
already validated.

Parse rates say a tree was produced, not that it is *correct*. To check
correctness, `oracle/` asks a running sclang what classes and methods it
actually compiled — names, class/instance, argument names, source positions —
and diffs that against what this crate extracts from the same files.

```bash
./oracle/run.sh
```

```
classes  oracle  1764   ours  1757      (7 missing, 0 extra, 0 superclass mismatches)
methods  oracle 14490   ours 14484      (6 missing, 0 extra)
argument-name mismatches: 0

methods matching sclang exactly (name and arguments): 14484 / 14490 (99.96%)
```

All 13 remaining differences come from two files that are not valid UTF-8 —
Latin-1 sources from the 2000s that this crate currently refuses to read. That
is the only known correctness gap.

The class library is open-ended: anyone can add classes via Extensions or
quarks. So there is no fixed set to check against and no useful fixture to
commit — the differ takes its file list from the live sclang dump, which means
it automatically covers whatever is installed on the machine it runs on. Re-run
it after installing a quark or upgrading SuperCollider.

A note on what this does *not* validate: it checks the symbol layer, which is
what an index needs. It does not compare expression structure. `DumpParseNode.cpp`
would give a full parse tree, but nothing in sclang ever calls `dump()` — no
flag, no primitive, no caller — so reaching it would mean patching and
rebuilding SuperCollider.

## Parse-tree dump

`DumpParseNode.cpp` has been in SuperCollider for years with no caller: no
flag, no primitive, nothing. `oracle/expose-dumpparsenode.patch` adds an
env-var hook so class-library compilation emits each file's parse tree.

```bash
./oracle/build-sclang-dump.sh          # clone, patch, build sclang only
SCLANG_DUMP_PARSE=1 <build>/lang/sclang -a -l conf.yaml -i none quit.scd
```

```bash
cargo run --release --example tree_oracle -- dump.log
```

```
methods compared  : 9,731
selectors compared: 53,684
selector sequences identical: 9,731 / 9,731 (100.00%)
```

Node-for-node comparison is impossible — the IR is desugared for a code
generator. What compares is the **pre-order sequence of selectors** in each
method body, which is sensitive to the thing that matters: `(1 + 2) * 3` emits
`*` then `+`, while `1 + (2 * 3)` emits `+` then `*`. Making the parser
right-associative drops it to 95.68%; emitting a selector after its arguments
instead of before drops it to 59.73%.

Reaching it needs a translation layer for the rewrites sclang performs while
parsing, each verified against the dump directly:

| source | sclang emits |
|---|---|
| `arr[3]` | `Call 'at'` |
| `arr[3] = 9` | `Call 'put'` |
| `arr[1..3]` | `Call 'copySeries'` |
| `(1..10)` | `Call 'prSimpleNumberSeries'` |
| `Point(1, 2)` | `Call 'new'` |
| `f(*args)` | `Call 'performList'` |
| `super.f(*args)` | `Call 'superPerformList'` |
| `obj.bar = 7` | nothing |
| `f.(1)` | `Call 'value'` |
| `~x` / `~x = 5` | `Call 'envirGet'` / `'envirPut'` |
| `arr[1..2] = x` | `Call 'putSeries'` |
| `Set[1, 2]` | a literal, no call |

Every divergence found along the way turned out to be a bug in sclang's
dumper rather than in this parser.

### Five bugs in sclang's own dumper

All bit-rot in code that has had no caller for years, and all had to be fixed
before the oracle was worth anything:

- `dumpPushLit` read its slot with `slotRawObject` where `newPyrPushLitNode`
  had filled it with `SetPtr` — the wrong union member. Function literals never
  dumped at all: zero `Func` lines across the class library. Fixing it yields
  13,400, and block interiors became comparable.
- `PyrMethodNode::dump` and `PyrBlockNode::dump` never emitted `mVarlist`, so
  `var x = this.foo` initialisers were invisible.
- Dumping the `PyrVarListNode` chain repeats declarations: the VarDef chain
  already spans every `var` statement, while each list after the first points
  partway along it. Dumping the defs directly fixes it.
- `PyrMethodNode::dump` never printed `mIsClassMethod`, so `*make` and `make`
  were indistinguishable — and classes routinely have both.
- `PyrCurryArgNode::dump` never dumped its `mNext`, alone among node types, so
  an argument list was truncated at the first `_` and everything after it
  vanished. This accounted for the last ten disagreements.

## Stability properties

The parser exists because sclang's own cannot handle incomplete input, so the
properties worth asserting are about edits, not about well-formed files.

Note that the naive idempotence property — parse, print, reparse, compare — is
*trivially* true for a lossless tree: printing returns the source byte for
byte, so reparsing is the same call twice. One test pins it as a guard, but it
finds nothing alone. What finds bugs is stability under truncation and
mutation:

```bash
cargo run --release --example stability -- <dir>...
```

```
files              : 682
prefixes parsed    : 357,716
mutations parsed   : 27,200
panics             : 0
losslessness breaks: 0
```

Every prefix of every file is what the parser sees on each keystroke, so
parsing all of them is a direct simulation of typing the class library from
scratch. Mutations add random single-character deletions and delimiter
insertions.

These have teeth: injecting a one-line bug into the tree builder (dropping
trailing tokens) fails 6 of the 8 idempotence tests and makes the sweep report
the exact prefixes affected.

## Tests

```bash
cargo test
```

The suite asserts specific token sequences for specific constructs, plus two
invariants that hold for *any* input: the stream is always lossless, and the
lexer never panics. Those two catch far more than the targeted cases do.
