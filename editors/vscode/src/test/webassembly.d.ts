// `vscode-oniguruma`'s declarations reference the `WebAssembly` namespace,
// which lives in `lib.dom.d.ts`. This extension runs in Node and deliberately
// leaves DOM out of `lib`, so the two names those declarations actually use
// are supplied here instead. Widening `lib` to fix one test dependency would
// put `document`, `window` and `fetch` in scope for extension code that has
// none of them, which is a worse trade than five lines.

declare namespace WebAssembly {
    type ImportValue = unknown;
    interface WebAssemblyInstantiatedSource {
        module: unknown;
        instance: unknown;
    }
}
