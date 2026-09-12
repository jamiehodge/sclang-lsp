# SuperCollider for VS Code, via `sclang-lsp`

Two halves that stay apart.

**Reading code** is served by [`sclang-lsp`](../../README.md): diagnostics as
you type, completion, signature help, inlay hints, hover, goto-definition,
references, rename, and document and workspace symbols. The server parses
SuperCollider itself, so all of it works with sclang absent, broken, or busy —
including on a class library that does not compile.

**Running code** is this extension's own job, in a child process it owns.
Evaluation, a post window, and the class-library compile errors that nothing
else can report. The server is not involved and never learns that sclang
exists.

## Running code

<kbd>⌘⏎</kbd> (<kbd>Ctrl+Enter</kbd>) evaluates, and starts sclang first if it
is not already up. What gets evaluated, in order:

1. the selection, if there is one;
2. otherwise the enclosing block — the outermost `( … )` whose opening
   parenthesis is at the start of a line, which is SuperCollider's own
   convention for a region;
3. otherwise the current line.

The block comes from the language server rather than from counting
parentheses, so `"("`, `$(`, `'('` and `// (` are text rather than structure,
and `SinOsc.ar(440)` is not mistaken for a region.

Output goes to a **SuperCollider** output channel — the post window, which is
just the child's stdout and stderr.

### One command, whatever bindings you want

`sclang-lsp.eval` takes the code to run as its argument, so there is no fixed
menu of server commands to learn. <kbd>⌘.</kbd> is bound to the hard stop out
of the box; anything else is a line in `keybindings.json`:

```json
{ "key": "cmd+b", "command": "sclang-lsp.eval", "args": "s.boot;",
  "when": "editorTextFocus && editorLangId == supercollider" }
```

`s.boot` is a line of SuperCollider. Wrapping it in a command of its own would
only add a second place for the server's state to be wrong.

### Class-library compile errors

sclang prints these while compiling, before any image exists:

```
ERROR: syntax error, unexpected '}'
  in file '/path/to/BadClass.sc'
  line 4 char 2:
```

Nothing that merely talks to a running sclang can see them — there is no
sclang running yet to ask. Holding the process is the only way to have them at
all, and they arrive in the Problems panel like any other diagnostic. Each
compile pass replaces the last one's.

### Why `-i none`

sclang's `-i` flag names the IDE, and it quietly decides something else too:
with any name but `none`, sclang assumes an IDE is driving it on another
channel and never starts its terminal reader. Nothing written to stdin is read,
and nothing says so — evaluation just does nothing. Since evaluation here *is*
stdin, the name is `none`, which is also the honest answer. Do not override it
through `sclang.sclang.args`.

### What to expect from a multi-line region

Over a pipe sclang reads one line at a time, so a multi-line region cannot be
written straight to stdin: it would be evaluated line by line and a block would
never close. Flattening it is worse, because the first `//` comment would
swallow the rest. So a region spanning more than one line is staged in a
scratch file and loaded, padded with the blank lines above it so the line
numbers in any error match the buffer you are looking at. The file name in a
stack trace is the scratch file, not your document.

Stopping sclang sends `0.exit` and waits. `Main.shutdown` runs `Server.quitAll`
on the way out, which is what keeps scsynth from being orphaned on the audio
device; force is the fallback, not the plan.

## Running it locally

From the repository root:

```bash
cargo build --release
```

Then, in this directory:

```bash
npm install
npm run compile
```

Open `editors/vscode` in VS Code and press <kbd>F5</kbd>. That launches an
Extension Development Host with the extension loaded; open any `.sc` or `.scd`
file in it.

No configuration is needed while developing: the extension looks for
`target/release/sclang-lsp` (then `target/debug`) in the checkout it lives in
before falling back to `PATH`.

[`sample.scd`](sample.scd) is a guided tour of every feature, in order, with a
deliberate syntax error at the end for the diagnostics.

```bash
npm test
```

covers the compile-error parser and the block picker. The sclang output format
belongs to sclang rather than to this project, so that fixture is recorded from
a real one rather than written to match the regexes. And

```bash
npm run e2e
```

runs the two checks that need real processes. One starts the built server over
stdio and feeds its real selection ranges through the same block picker, so the
chains the unit tests assert against are checked rather than assumed. The other
starts a real sclang, writes to its stdin and waits for the answer.

That second one exists because of a bug it would have caught: the extension
once shipped with `-i vscode`, and nothing short of a live sclang could have
noticed, since the failure is entirely on the far side of the pipe. It takes
its arguments from `src/launch.ts`, the same module the extension uses — a
test holding its own copy of them would have passed throughout. It skips when
SuperCollider is not installed, which is why it is not part of `npm test`.

## Installing it

To use it in your normal editor rather than a development window:

```bash
cargo build --release          # from the repository root
npm install && npm run package # bundles the binary into the VSIX
code --install-extension sclang-lsp-0.2.2.vsix
```

`npm run package` copies `target/release/sclang-lsp` into `server/` first. An
installed extension lives in `~/.vscode/extensions` with no repository near it,
so without the bundled copy there is nothing for it to find and you would have
to set `sclang-lsp.server.path` by hand. Bundling makes the VSIX specific to
the platform it was built on, which is what the per-target release artifacts
exist for.

> **Disable any other SuperCollider extension first.** Two extensions
> contributing the `supercollider` language means two servers answering every
> request, and duplicate completions and hovers.

## Settings

| Setting | Effect |
|---|---|
| `sclang-lsp.server.path` | Absolute path to the server binary. Overrides the search above. |
| `sclang-lsp.classLibraryPaths` | Index these directories *instead of* the platform defaults. Leave empty to let the server find the stock class library, `Extensions` and `downloaded-quarks`. |
| `sclang-lsp.trace.server` | Log LSP traffic. `messages` or `verbose` for debugging. |
| `sclang-lsp.sclang.path` | Absolute path to `sclang`. Empty means the usual place for the platform, then `PATH`. |
| `sclang-lsp.sclang.args` | Extra arguments for `sclang`, after `-i none`. `-l` goes here to point at a `sclang_conf.yaml`. |
| `sclang-lsp.echo` | Echo evaluated code into the post window. On by default. |
| `sclang-lsp.compileErrors` | Publish class-library compile errors as diagnostics. On by default. |

The first three restart the language server when changed. The sclang ones are
read each time it is launched, so they take effect on the next restart of it.

## Two editor behaviours worth knowing

**Signature help is not shown just because the cursor is inside a call.** VS
Code requests it when you type `(` or `,`, or when you ask for it with ⌘⇧Space
(*Trigger Parameter Hints*) — not when the cursor arrives in parens that are
already closed. The server answers at every position inside a call; the editor
simply does not ask on cursor movement.

**Inlay hints follow `editor.inlayHints.enabled`**, which some setups turn off
or set to `onUnlessPressed`. They need no cursor and appear on their own.

## What it does not do

No node tree, no level meters, no scope. Those need data pushed back out of a
running image, which means shipping SuperCollider code alongside the extension
— and the moment there is sclang-side code there is a second thing to install,
version and keep compiling. `s.meter` still works; it opens its own window.

No `~envir` completion either. Environment variables live in a running image
and nothing static can enumerate them, which is the one real capability given
up by a server that never contacts one.

**Do not enable this alongside another SuperCollider extension.** Two
extensions contributing the `supercollider` language means two servers
answering, and duplicate completions and hovers.

## Licensing

The extension is GPL-3.0-or-later, matching the server.

`language-configuration.json` and `syntaxes/supercollider.tmLanguage.json` are
taken from [`vscode-supercollider`](https://github.com/scztt/vscode-supercollider),
MIT © 2022 Scott Carver. See [LICENSE-MIT](LICENSE-MIT).
