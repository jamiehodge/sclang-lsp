// Choosing the block from a chain of selection ranges.
//
// No editor API, so the rule can be tested directly. The chain itself comes
// from the language server, which is what makes this sound: the parentheses in
// it are the ones the grammar saw, so `"("`, `$(`, `'('` and `// (` are text
// rather than structure.

/** One step of a selection-range chain, innermost first. */
export interface Step {
    text: string;
    /** Column of the step's first character, zero-based. */
    column: number;
}

/**
 * The index of the outermost parenthesised block, or `undefined` for none.
 *
 * SuperCollider's convention is that a region is delimited by parentheses at
 * the start of a line, and that qualifier is load-bearing rather than
 * decorative: without it the argument list in `SinOsc.ar(440)` reads as a
 * block, since it is also a parenthesised step in the chain.
 *
 * Outermost rather than innermost, so a cursor inside a nested `( … )` still
 * evaluates the whole region — which is what the SuperCollider IDE does.
 */
export function pickBlock(steps: Step[]): number | undefined {
    let found: number | undefined;

    for (let i = 0; i < steps.length; i += 1) {
        const step = steps[i];
        if (step.column === 0 && step.text.startsWith('(') && step.text.endsWith(')')) {
            found = i;
        }
    }

    return found;
}
