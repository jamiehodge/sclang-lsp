# Changelog

Notable changes, newest first. Versions follow [semver](https://semver.org),
with the usual pre-1.0 caveat that the minor number carries breaking changes.

## Unreleased

### Added

- **The class library is found the way sclang finds it.** `sclang_conf.yaml`
  is where sclang is told what to compile, and `includePaths` is how anyone
  developing a quark points at their own checkout of it. The guessed locations
  never saw those, so the server quietly indexed less than sclang did, and
  nothing said so — goto-definition simply had no answer for a class that was
  right there. The file is read now, along with `excludePaths` (by prefix, so
  excluding a quark excludes what is under it) and `excludeDefaultPaths`.

  Reading it is not a dependency on a running sclang, any more than reading a
  `.sc` file is. An explicit `classLibraryPaths` still replaces the lot.

  Measured on a machine with one quark checked out under `~/Documents`: 42 more
  files, 6 more classes, 116 more methods. The resolution oracle, which had
  needed those roots passed by hand to agree, now agrees on 751,601 of 751,639
  pairs without being told anything.

- A root that another root already contains is dropped, so a quark named in
  `sclang_conf.yaml` *and* sitting in `downloaded-quarks` is walked once rather
  than twice.

### Fixed

- **A node with no children no longer claims the wrong range.** The tree
  builder gave one `0..0` regardless of where it was opened, and a node takes
  its span from its first and last child — so a parent whose first child was an
  empty node started at the top of the file, and one whose last child was
  empty *ended* there, before it began. `Foo { var a = #; }` was enough:
  reading that node's text panicked the indexer, which runs on every keystroke
  and on the server's own thread, so the editor lost its language server. Two
  more crashes had the same cause, in folding ranges and in scope lookup, and
  the quiet cases were worse — an empty node at the end of a file collapsed the
  root's range, and every feature that asks what is at a position went blind
  for the whole buffer.

  An empty node now keeps the zero-width range it was opened at. The parser
  tests check the invariant directly, exhaustively over every four-character
  input, and a new `robustness.rs` asks every feature at every position in
  several thousand mutated files.

- **One stray `)` in a class body is one diagnostic, not 257.** Neither `)`
  nor `]` is a class member, a recovery target, or something the recovery
  scan will step over, so it left the cursor where it was and the member loop
  asked again from the same place — until the parser's fuel ran out 256
  iterations later and something finally moved. The problems panel filled with
  identical entries.

- **A scratch buffer no longer replaces a real class.** `Routine { … }` is a
  class definition in a `.sc` file and a call in a script, and only the file
  name says which — but the indexer always read it the first way. Opening a
  `.scd` beginning `Routine { 1.rand }.play` therefore declared a class named
  `Routine`, which replaced the class library's: its superclass became
  `Object`, and goto-definition on `Routine` anywhere in the workspace landed
  in the scratch buffer. Document symbols listed the phantom too. The mode now
  follows the file name everywhere it is decided, through one function.

- **Rename refuses a reserved word.** `var`, `arg`, `nil`, `true`, `pi` and
  `inf` all pass a letters-and-digits test and none of them is a name, so
  renaming a local to one wrote `var var = 1;` into the file — a silent syntax
  error, which is the one thing that module exists to avoid. The new name is
  now lexed and has to come out as a single token of the right kind, which
  also stops the check being *stricter* than the language: sclang's character
  table puts a non-ASCII letter in the identifier class, so `café` is a name
  and was being refused.

- **An inlay hint no longer counts a `;` as an argument.** An argument is an
  `exprseq`, so `SinOsc.ar(1; 0)` passes one argument whose value is `0`.
  Counting the halves separately put the second parameter's label on the
  semicolon, and `SinOsc.ar(1;)` grew a label for an argument that is not
  there. Nothing is labelled past a `*args` either: it spreads across every
  remaining parameter, so no position after it is a fact.

- **Inherited slots stop at the end of the class body.** The slots of the
  class being walked come from the index and were left in place on the way
  out, so a name written after the class was painted as a property of a class
  it is not in. Loose code after an unbalanced brace is the ordinary state of a
  file being typed into.

- **`=` is an operator in a declaration too.** Only the assignment form was
  listed, so the same character was coloured in `x = 1` and left alone in
  `var x = 1` two lines away.

- **The evaluated-region flash lasts its full quarter second.** A second ⌘⏎
  within it was cleared by the first one's timer, so the second region barely
  lit up — and ⌘⏎ twice in quick succession is how the thing is used.

- **An array element may end with a `;`.** `arrayelems1 : exprseq` and
  `exprseq : exprn optsemi`, so the `;` before a `]` is allowed — and it is not
  a curiosity anyone invented: `ScIDE.sc`, in the stock class library, ends an
  array element with one, and six of its lines did not parse. Argument lists
  and event literals already took a trailing `;`; array literals did not. Found
  by running the conformance sweep against a real install, which is also what
  confirmed the fix: every `.sc` class file in the corpus parses again, and the
  six remaining failures are `.scd` scripts that sclang rejects too.

- **A typed collection literal names its class.** `Set[1, 2].includes(x)` had
  an unknown receiver and offered every `includes` in the image. The code said
  otherwise — "`Set[...]` names its own class" — but looked for the name on a
  `Collection` node, and `msgsend : classname '[' arrayelems ']'` is read
  through the postfix chain, so it arrives as an index on a class reference and
  a `Collection` never carries a class name at all. That branch could not have
  fired. `IdentityDictionary[…]`, `List[…]` and `#Set[…]` resolve too.

- **Three constructs the grammar has and the parser did not.** Found by reading
  `grammar/sclang.y` against `grammar.rs` production by production, and each
  confirmed against sclang 3.13 before being changed:
  `classname '[' arrayelems ']'` takes the `key: value` forms, so `Set[0: 1]`
  parses; `listlit` has a second spelling that names the class, so `#Set[1, 2]`
  parses; and `expr binop2 adverb expr` covers `binop2 : binop | keybinop`, so
  the adverb in `a foo: .x b` parses. None of the three occurs anywhere in a
  stock install, which is why no sweep had found them.

- **`2pi` is a Float, not an Integer.** `floatp : integer pie` — a `pi`
  suffix makes the whole literal a float, and the lexer gives `2pi` as two
  tokens, so reading only the first called it an Integer. It called it that
  with `Certain` behind it, which is the tier that may be rendered into the
  buffer: an inlay hint would have written Integer's parameter names into a
  send to a Float. `pi` on its own had no class at all.

- **`true`, `false` and `nil` have a class.** Each is the sole instance of
  one, which is as much a fact of the grammar as a string literal being a
  String — but the table of literal classes had rows for the rest and not for
  these, so `nil.isNil` resolved to every `isNil` in the image rather than to
  `Nil`'s. An accidental like `4s` is still left alone on purpose.

- **Hover reads a default written without an `=`.** `optequal` really is
  optional — `|range -1|` and `|overwrite(true)|` both declare a default — and
  the index had always read both while the scope walk had not, so hover and
  signature help contradicted each other about the same parameter.

## [0.11.0] — 2026-09-16

### Added

- **`this` and `super` resolve.** A send to `this` is a quarter of the sends in
  the stock class library — 8,419 of 37,707 — and the class it names is written
  at the top of the node the cursor is already inside. Reading it off the tree
  is not inference; it is the same act as reading `String` off a string
  literal. Goto, hover, completion and signature help all narrow accordingly:
  `this.sound` inside `Dog` goes to `Dog`'s, not to a list of every class with
  a `sound`. In a class method `this` is the class object, so `^this.multiNew`
  is a class-side send; `super` starts one class up on the same side.

  Narrowing never turns an honest list into nothing. 1.6% of `this.` sends in
  the class library name a selector the chain does not define — a superclass
  calling a method only its subclasses implement, like
  `SequenceableCollection`'s `choose` sending `this.at` — so a chain miss falls
  back to every implementor, which is exactly the answer it gave before.

  `super` inside a `+ Foo { }` extension stays unknown: an extension header
  writes down no superclass, and guessing beats saying nothing at nobody's
  expense. Three sends in the stock library.

- **Inherited class slots.** `Pgate` reads `pattern`, an instance variable
  declared two classes above it on `FilterPattern` and in another file, so the
  lexical walk over one buffer never saw it — no hover, no goto, no completion,
  and the wrong colour. The index had the answer the whole time and nothing
  asked for it. In the stock class library 2,868 name uses resolve this way:
  2,717 declared on an ancestor, and 151 a class's own slots read from inside a
  `+ Foo { }` extension, which declares none of its own.

  Hover names the class the slot comes from, goto follows it into that class's
  own file, completion offers it with the same label, and semantic tokens paint
  it as a property. A `var` with no `<` marker generates no accessor, so this
  is the only way such a slot is reachable at all. Anything the buffer declares
  still shadows it, exactly as at run time.

  Find-references is deliberately not included. A slot's uses are plain
  identifiers spread across every subclass in the workspace, and the occurrence
  index records only class names and selectors; an answer covering the open
  buffer alone would read as complete and would not be.

- **Certainty, so an inlay hint can say more without claiming more.** The class
  of a receiver was already known in two quite different senses — the grammar
  settling a literal's class, and the convention that `*new` returns an
  instance of its own class — and the two were collapsed into one answer, so
  the features that render into the buffer had to refuse both. They are now
  kept apart. `"abc".copyRange(0, 2)` gets its parameter names labelled;
  `Thing.new.at(1, 2)` still does not, because a class is free to return
  something else from `*new` and a few do.

- **A variable carries the class it was initialised with.** `var pet = Cat.new`
  makes `pet.sound` resolve to `Cat`'s, on the same convention that already
  covered `Cat.new.sound` written inline. Nothing follows an assignment, and
  nothing chains through a second variable — that would be inference proper.

  Argument defaults are deliberately excluded. `|pet = Cat.new|` says what
  happens when the parameter is *not* passed; every caller remains free to pass
  anything, so reading it as the class of `pet` would be a claim about them.

- **Completion after a `.` is ranked by distance up the chain.** Narrowing the
  list was not enough to make it *look* narrowed: `Pbind` declares four
  instance methods and inherits 437, so a correctly scoped list of 441 was hard
  to tell from the unscoped 6,178. The receiver's own methods now sort first,
  then its superclass's, and so on — `SinOsc.` opens on `ar` and `kr` rather
  than on whatever `Object` has beginning with A.

  This only decides the order when nothing has been typed yet, which is exactly
  when there is no better signal. Once there is a prefix the client's own match
  scoring leads. An unknown receiver has no chain to rank against and is left
  alone.

### Fixed

- A send to a class object now falls back to `Class`'s instance side before the
  class's own, so `this.allSubclasses` and `this.name` inside a `*method`
  resolve to where they actually live.

## [0.10.0] — 2026-09-15

### Added

- **Formatting.** `textDocument/formatting` and `textDocument/rangeFormatting`,
  the second being the "re-indent this region" the SuperCollider IDE binds to a
  key.

  It is an indenter, not a formatter: it rewrites the leading whitespace of a
  line and never moves a line break. SuperCollider's two most-written idioms
  are hand-aligned columns — a `Pbind`'s key/value pairs, a `SynthDef`'s UGen
  arguments — and reflowing either collapses them onto one line or explodes
  them to one item per line. There is no agreed SuperCollider style to converge
  on either, so choosing line breaks would mean minting one.

  The editor's own indentation is two regexes, wrong on `"("`, `$(`, `'('` and
  `// (`, unable to track a nested block comment, and unable to tell `|a, b|`
  from `a | b`. The parser is wrong about none of them.

  Every answer is relative to an earlier line: a body is one level deeper than
  the line its bracket opened on, a closing bracket takes that line exactly,
  and a continuation keeps the offset it already had — so a column you aligned
  by hand survives and still moves with its block.

  Tabs versus spaces is the editor's `FormattingOptions`, never ours. A file
  that does not parse is left alone, and silently, because format-on-save runs
  on every save of a file you are still typing into.

## [0.9.0] — 2026-09-13

### Added

- **Folding ranges.** Regions, class and method bodies, function literals,
  collections and block comments.

  The editor's fallback is indentation, and SuperCollider's central idiom
  defeats it: a region is `(` and `)` at the start of a line, and what sits
  between them is routinely not indented at all. There is no shape to infer a
  fold from, so the one construct a file is organised around was the one that
  could not be collapsed. The parser knows where each region ends.

  `//#region` and `//#endregion` are honoured too. Declaring a folding range
  provider is what stops the editor handling those markers itself, so leaving
  them out would have quietly broken something that worked. They are read off
  comment tokens rather than off lines, so a `//#region` inside a string is not
  mistaken for one — which the editor's own marker matching would do.

  Ranges are line-based, and so need no position encoding: how a client counts
  characters changes the column and never the line.

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

[0.11.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.11.0
[0.10.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.10.0
[0.9.0]: https://github.com/jamiehodge/sclang-lsp/releases/tag/v0.9.0
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
