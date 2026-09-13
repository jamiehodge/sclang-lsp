# SuperCollider for VS Code

Language features for SuperCollider, and an sclang to run your code in.

**Reading code** — completion that knows the class library, hover with real
signatures, goto-definition, find-references, rename, symbols, inlay hints,
parse-accurate syntax colouring, and syntax errors as you type. All of it comes
from parsing, so it works on code that does not compile and while sclang is
busy.

**Running code** — evaluate a selection, a block or a line; watch the post
window; and see class-library compile errors in the Problems panel.

## Install

Download the `.vsix` for your platform from the
[releases page](https://github.com/jamiehodge/sclang-lsp/releases/latest) —
`darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64` or `win32-x64` — and
install it:

```bash
code --install-extension sclang-lsp-darwin-arm64.vsix
```

The server binary is bundled inside, so nothing needs configuring.
SuperCollider itself is only needed if you want to run code.

Not on the Marketplace yet. [DEVELOPING.md](DEVELOPING.md) covers building it
from source.

> Disable any other SuperCollider extension first. Two extensions contributing
> the `supercollider` language means two servers answering every request, and
> duplicate completions and hovers.

## Evaluating

<kbd>⌘⏎</kbd> (<kbd>Ctrl+Enter</kbd>) evaluates, and starts sclang the first
time. What it sends, in order:

1. the selection, if there is one;
2. otherwise the enclosing block — the outermost `( … )` whose opening
   parenthesis is at the start of a line;
3. otherwise the current line.

The block is found by parsing rather than by counting parentheses, so `"("`,
`$(` and `// (` are text rather than structure, and `SinOsc.ar(440)` is never
mistaken for a region.

Output appears in a **SuperCollider** output channel — the post window.

### Your own bindings

`sclang-lsp.eval` takes the code to run as its argument, so there is no fixed
menu of commands to learn. <kbd>⌘.</kbd> is bound to the hard stop already;
anything else is a line in `keybindings.json`:

```json
{ "key": "cmd+b", "command": "sclang-lsp.eval", "args": "s.boot;",
  "when": "editorTextFocus && editorLangId == supercollider" }
```

`s.boot` is a line of SuperCollider. Wrapping it in a command of its own would
only add a second place for the server's state to be wrong.

### Compile errors

When the class library fails to compile, the errors land in the Problems panel
with the file and line, and each compile replaces the last one's.

sclang prints these before any image exists, so an extension that holds its
output is the only thing that can report them at all.

### Stopping cleanly

Three paths, because one is not enough:

- **Closing the window** runs `deactivate`, which waits for sclang to leave
  properly — `0.exit`, so `Server.quitAll` runs and scsynth releases the audio
  device on the way out.
- **The extension host exiting under it** hits a synchronous kill registered on
  `process.exit`, since `dispose` returns void and nothing waits for it.
- **The host being killed outright** closes sclang's stdin, and sclang exits on
  EOF by itself.

An sclang that does get left behind is not a quiet nuisance: with nothing
holding its stdin it spins at 100% of a core, and the first symptom is usually
not a sluggish editor but dropouts in whatever is playing, because scsynth is
competing with it for CPU.

## Commands

| | |
|---|---|
| `Evaluate selection or block` | <kbd>⌘⏎</kbd> |
| `Evaluate code` | Takes its code as an argument, for your own bindings |
| `Start sclang` / `Stop sclang` / `Restart sclang` | |
| `Show post window` | |
| `Restart language server` / `Show server log` | |

## Settings

| Setting | Effect |
|---|---|
| `sclang-lsp.server.path` | Absolute path to the server binary. Defaults to the copy bundled here. |
| `sclang-lsp.classLibraryPaths` | Index these directories instead of the platform defaults. |
| `sclang-lsp.trace.server` | Log LSP traffic — `messages` or `verbose`. |
| `sclang-lsp.sclang.path` | Absolute path to `sclang`. Empty means the usual place for your platform, then `PATH`. |
| `sclang-lsp.sclang.args` | Extra arguments for `sclang`. Do not override `-i`: under any IDE name but `none`, sclang never reads stdin and evaluation silently does nothing. |
| `sclang-lsp.echo` | Echo evaluated code into the post window. On by default. |
| `sclang-lsp.compileErrors` | Compile errors as diagnostics. On by default. |

The first three restart the language server; the sclang ones apply next time it
starts.

## Two behaviours worth knowing

**Signature help is not shown just because the cursor is inside a call.** VS
Code asks for it when you type `(` or `,`, or when you press ⌘⇧Space
(*Trigger Parameter Hints*) — not when the cursor arrives in parens that are
already closed. The server answers at every position inside a call; the editor
simply does not ask on cursor movement.

**Inlay hints follow `editor.inlayHints.enabled`**, which some setups turn off.
They need no cursor and appear on their own.

**Errors in a multi-line block name a scratch file**, not your document,
because the block is staged in a file to reach sclang intact. Line numbers
still match your buffer.

## Not included

No node tree, no level meters, no scope. Those need data pushed back out of a
running image, which means shipping SuperCollider code alongside the extension
— and then there is a second thing to install, version and keep compiling.
`s.meter` still works; it opens its own window.

No `~envir` completion either. Environment variables live in a running image
and nothing static can enumerate them.

## Building on it

[DEVELOPING.md](DEVELOPING.md) covers the `F5` loop, the tests, and packaging.

[`sample.scd`](sample.scd) is a guided tour of every language feature, in
order, with a deliberate syntax error at the end for the diagnostics.

## License

GPL-3.0-or-later, matching the server.

`language-configuration.json` and `syntaxes/supercollider.tmLanguage.json` are
taken from [`vscode-supercollider`](https://github.com/scztt/vscode-supercollider),
MIT © 2022 Scott Carver. See [LICENSE-MIT](LICENSE-MIT).

The grammar still does the first pass — it is synchronous, and it paints before
the server has started. The server's semantic tokens arrive after and refine
it, which is what tells a selector from a variable and a parameter from a
local. VS Code layers the two by default; `editor.semanticHighlighting.enabled`
turns the second one off.
