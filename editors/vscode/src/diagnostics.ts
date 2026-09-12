// Compile errors as editor diagnostics.
//
// These are printed by the compiler before any image exists, which is why
// nothing that merely talks to a running sclang can report them, and why
// holding the process is the only way to have them at all.

import * as vscode from 'vscode';
import { CompileErrorParser } from './compile-errors';

export class CompileErrors implements vscode.Disposable {
    private readonly collection = vscode.languages.createDiagnosticCollection('sclang');
    private readonly parser = new CompileErrorParser();
    private readonly found = new Map<string, vscode.Diagnostic[]>();

    feed(chunk: string): void {
        const batch = this.parser.feed(chunk);

        if (batch.reset) {
            this.clear();
        }

        for (const error of batch.errors) {
            const start = new vscode.Position(error.line, error.character);
            const diagnostic = new vscode.Diagnostic(
                // The compiler gives a point, not a span. Running to the end
                // of the line claims no more than it knows.
                new vscode.Range(start, start.with({ character: Number.MAX_SAFE_INTEGER })),
                error.message,
                vscode.DiagnosticSeverity.Error,
            );
            diagnostic.source = 'sclang';

            const existing = this.found.get(error.file) ?? [];
            existing.push(diagnostic);
            this.found.set(error.file, existing);
            this.collection.set(vscode.Uri.file(error.file), existing);
        }
    }

    clear(): void {
        this.found.clear();
        this.collection.clear();
    }

    dispose(): void {
        this.collection.dispose();
    }
}
