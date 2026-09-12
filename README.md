# sclang-lsp

A language server foundation for SuperCollider, derived from sclang's own front end.

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
| parser (next) | `Bison/lang11d` — the bison grammar |

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

## Status

- [x] Lexer — 35 tests, zero error tokens over the full class library
- [x] Parser — recursive descent from `lang11d`, with error recovery
- [x] CST — lossless, round-trips every file in the corpus
- [x] Differential oracles — token-for-token against sclang's own lexer,
      symbol-for-symbol against its compiled class library
- [ ] Symbol index
- [ ] LSP server

## Conformance

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

## Tests

```bash
cargo test
```

The suite asserts specific token sequences for specific constructs, plus two
invariants that hold for *any* input: the stream is always lossless, and the
lexer never panics. Those two catch far more than the targeted cases do.

## License

GPL-3.0-or-later, matching SuperCollider, since this is a derived work of its
front end.
