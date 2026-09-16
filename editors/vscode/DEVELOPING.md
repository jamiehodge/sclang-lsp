# Developing the extension

## The loop

From the repository root:

```bash
cargo build --release
```

Then here:

```bash
npm install
npm run compile
```

Open `editors/vscode` in VS Code and press <kbd>F5</kbd>. That launches an
Extension Development Host with the extension loaded; open any `.sc` or `.scd`
file in it.

Nothing needs configuring while developing. The extension looks for
`target/release/sclang-lsp`, then `target/debug`, in the checkout it lives in,
before falling back to `PATH`.

## Tests

```bash
npm test
```

Unit tests, no editor and no SuperCollider required. They cover the two pieces
of real logic: `block.ts`, which picks the region to evaluate out of a chain of
selection ranges, and `compile-errors.ts`, which reads sclang's error format.
That second fixture is output recorded from a real sclang rather than written
to match the regexes, because the format belongs to sclang and not to this
project.

```bash
npm run e2e
```

Two checks that need real processes. The first starts the built server over
stdio and feeds its actual selection ranges through the same block picker, so
the chains the unit tests assert against are verified rather than assumed. The
second starts a real sclang, writes to its stdin and waits for the answer; it
skips when SuperCollider is not installed, which is why it is not part of
`npm test`.

## Two things the pipe imposes

Both were found by testing against a real sclang, and neither is visible from
inside the extension.

**`-i` decides whether sclang reads stdin at all.** It names the IDE, but under
any name except `none` sclang assumes an IDE is driving it on another channel
and never starts its terminal reader. Everything written to stdin is discarded
with no error, no prompt and nothing in the post window. Evaluation here *is*
stdin, so the name is `none` — which is also the honest answer, since this is
not the Qt IDE and provides none of its primitives.

This is why `e2e-sclang.js` takes its arguments from `src/launch.ts`, the same
module the extension uses. A test holding its own copy of them would assert its
own copy, and would have passed throughout the bug above.

**sclang reads a line at a time.** A multi-line region cannot go down the pipe
as written — it would be evaluated line by line and a block would never close.
Flattening it is worse, because the first `//` comment would swallow the rest.
So anything spanning more than one line is staged in a file and loaded, padded
with the blank lines above it so the line numbers in an error match the buffer
the user is looking at.

## Packaging

```bash
cargo build --release          # from the repository root
npm install && npm run package
code --install-extension sclang-lsp-*.vsix
```

`npm run package` copies `target/release/sclang-lsp` into `server/` first. An
installed extension lives in `~/.vscode/extensions` with no repository near it,
so without the bundled copy there is nothing for it to find.

That makes the `.vsix` specific to the platform it was built on. The release
workflow handles this properly: each of the five server targets also produces a
VS Code platform-specific build, tagged with `vsce package --target`, so
installing from the Marketplace or the releases page gets the right one.
`scripts/bundle-server.js` takes an optional directory for that, since a cross
compile lands in `target/<triple>/release` rather than `target/release`.

## Two builds, and why

`tsc` and esbuild both turn `src/` into JavaScript, and both are wanted:

| | |
|---|---|
| `npm run compile` | `tsc` into `out/`. Typechecks, and is what the tests run against. |
| `npm run bundle` | esbuild into `dist/extension.js`. One file, which is what `main` points at and what ships. |

esbuild strips types without checking them, so it is never the only step:
`vscode:prepublish` runs `compile` first, and a type error fails the package
rather than being bundled past. The F5 task depends on `compile` for the same
reason.

Bundling is what keeps the `.vsix` to 13 files. `vscode-languageclient` and its
dependencies are 315 files of JavaScript that get inlined into the one, so
`.vscodeignore` drops `node_modules` entirely — nothing in the bundle requires
anything but Node builtins and `vscode`.

It is deliberately not minified. The saving is about 440 KB against a package
the 2.8 MB server binary already dominates, and the cost would be a stack trace
nobody can read in the one place — a user's log — where this extension has to
explain what sclang did.

`bundle:dev` adds a source map so F5 breakpoints land in the TypeScript. The
published bundle has none: the map is build output, `.vscodeignore` drops it,
and shipping the reference without the file would be a dangling
`sourceMappingURL`.

## Layout

| | |
|---|---|
| `extension.ts` | Activation, commands, and the language client |
| `sclang.ts` | The child process, the post window, and stdin |
| `launch.ts` | Finding sclang and the arguments to start it with |
| `region.ts` | Selection, else enclosing block, else current line |
| `block.ts` | Picking the block out of a chain of selection ranges |
| `compile-errors.ts` | Reading sclang's error format out of the stream |
| `diagnostics.ts` | Publishing those as editor diagnostics |

`launch.ts`, `block.ts` and `compile-errors.ts` import no editor API, so they
can be tested directly — which is the whole reason they are separate.
