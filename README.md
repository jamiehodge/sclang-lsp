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
- [ ] Symbol index
- [ ] LSP server

## Conformance

The lexer is checked against real SuperCollider source in bulk — the class
library, plus installed Extensions and quarks:

```
files              : 682
bytes              : 2,501,312
tokens             : 804,840
error tokens       : 0 (0.0000%)
lossless (tokens)  : ALL FILES
---- parser ----
files parsed clean : 644 / 682 (94.43%)
classes found      : 2,074
methods found      : 12,260
lossless (tree)    : ALL FILES
```

The tree is lossless on *every* file, including the 38 with syntax the parser
does not yet cover — error recovery keeps the rest of those files intact, which
is the property that matters for an editor.

Reproduce with:

```bash
cargo run --release --example conformance -- \
  /Applications/SuperCollider.app/Contents/Resources/SCClassLibrary \
  ~/Library/Application\ Support/SuperCollider/Extensions
```

Note that lexing is a substantially lower bar than parsing; a clean sweep here
means the token model is faithful, not that the language is fully handled.

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
