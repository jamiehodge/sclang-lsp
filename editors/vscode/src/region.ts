// What to evaluate: the selection, else the enclosing block, else the line.
//
// The middle case is the only hard one, and it is a parsing question rather
// than a text one. So the candidate ranges come from the language server's
// `textDocument/selectionRange`, and `block.ts` picks one.

import * as vscode from 'vscode';
import { pickBlock, Step } from './block';

export interface Region {
    text: string;
    /** Where it came from, for the flash and for line-accurate errors. */
    range: vscode.Range;
}

export async function regionAt(editor: vscode.TextEditor): Promise<Region | undefined> {
    const doc = editor.document;
    const selection = editor.selection;

    if (!selection.isEmpty) {
        return { text: doc.getText(selection), range: selection };
    }

    const block = await enclosingBlock(doc, selection.active);
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
): Promise<Region | undefined> {
    const chains = await vscode.commands.executeCommand<vscode.SelectionRange[]>(
        'vscode.executeSelectionRangeProvider',
        doc.uri,
        [position],
    );

    // No provider means the language server is not running. Blocks are its
    // contribution; without it the caller falls back to the current line.
    if (!chains?.length) {
        return undefined;
    }

    // Flatten the parent links into the innermost-first list `pickBlock` wants.
    const ranges: vscode.Range[] = [];
    for (let node: vscode.SelectionRange | undefined = chains[0]; node; node = node.parent) {
        ranges.push(node.range);
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
