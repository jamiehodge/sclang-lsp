// What to evaluate: the selection, else the enclosing block, else the line.
//
// The middle case is the only hard one, and it is a parsing question rather
// than a text one. So the candidate ranges come from the language server's
// `textDocument/selectionRange`, and `block.ts` picks one.
//
// The server is asked *directly* rather than through
// `vscode.executeSelectionRangeProvider`. That command merges every registered
// provider with VS Code's own `WordSelectionRangeProvider`, whose last
// contribution is unconditionally `getFullModelRange()` — the whole buffer. In
// a file that begins with `(` and ends with `)`, which is any file of stacked
// regions with no trailing newline, that step is shaped exactly like a region,
// and `pickBlock` takes the outermost one. So ⌘⏎ evaluated the entire file.
//
// Asking the client is also the more honest question: what is wanted here is
// what the parser saw, not what every provider in the editor thinks.

import * as vscode from 'vscode';
import {
    LanguageClient,
    SelectionRange as ProtocolSelectionRange,
    SelectionRangeRequest,
} from 'vscode-languageclient/node';
import { pickBlock, Step } from './block';

export interface Region {
    text: string;
    /** Where it came from, for the flash and for line-accurate errors. */
    range: vscode.Range;
}

export async function regionAt(
    editor: vscode.TextEditor,
    client: LanguageClient | undefined,
): Promise<Region | undefined> {
    const doc = editor.document;
    const selection = editor.selection;

    if (!selection.isEmpty) {
        return { text: doc.getText(selection), range: selection };
    }

    const block = client && (await enclosingBlock(doc, selection.active, client));
    if (block) {
        return block;
    }

    // No block around the cursor, so the line is the region. A blank one is
    // not worth sending.
    const line = doc.lineAt(selection.active.line);
    return line.text.trim().length > 0 ? { text: line.text, range: line.range } : undefined;
}

async function enclosingBlock(
    doc: vscode.TextDocument,
    position: vscode.Position,
    client: LanguageClient,
): Promise<Region | undefined> {
    // Positions go over as they are: the client advertises `utf-16` as the
    // only encoding it accepts, so the server is already speaking VS Code's.
    const chain = await client
        .sendRequest(SelectionRangeRequest.type, {
            textDocument: { uri: doc.uri.toString() },
            positions: [{ line: position.line, character: position.character }],
        })
        // A server that is starting, stopping or gone answers nothing.
        // Blocks are its contribution, so the caller falls back to the line.
        .catch(() => undefined);

    if (!chain?.length) {
        return undefined;
    }

    // Flatten the parent links into the innermost-first list `pickBlock` wants.
    const ranges: vscode.Range[] = [];
    for (let node: ProtocolSelectionRange | undefined = chain[0]; node; node = node.parent) {
        ranges.push(
            new vscode.Range(
                node.range.start.line,
                node.range.start.character,
                node.range.end.line,
                node.range.end.character,
            ),
        );
    }

    const steps: Step[] = ranges.map((range) => ({
        text: doc.getText(range),
        column: range.start.character,
    }));

    const index = pickBlock(steps);
    return index === undefined
        ? undefined
        : { text: steps[index].text, range: ranges[index] };
}
