<!--
Thanks for the patch. Nothing here is mandatory — delete what does not apply.
CONTRIBUTING.md has the details.
-->

## What this changes

## Why

<!--
For a change to the lexer or parser, the question that matters is what sclang
itself does. If `lang/LangSource` settled it, say where.
-->

## Checks

- [ ] `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `npm test` in `editors/vscode`, if the extension changed
- [ ] The oracles, if this could change what is parsed — see CONFORMANCE.md
