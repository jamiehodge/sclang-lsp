// The chains here are the ones sclang-lsp actually returns; `e2e-selection.js`
// checks that claim against the running server.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { pickBlock, Step } from '../block';

/** A chain, innermost first, as `[column, text]` pairs. */
function steps(...pairs: [number, string][]): Step[] {
    return pairs.map(([column, text]) => ({ column, text }));
}

test('picks the parenthesised block at the start of a line', () => {
    const chain = steps(
        [11, '440'],
        [10, '(440)'],
        [1, 'SinOsc.ar(440)'],
        [0, '(\n\tSinOsc.ar(440);\n)'],
        [0, '(\n\tSinOsc.ar(440);\n)\n'],
    );

    assert.equal(pickBlock(chain), 3);
});

test('an argument list is not a block', () => {
    // `SinOsc.ar(440)` with no enclosing region. `(440)` is parenthesised and
    // in the chain, but it does not start a line, and evaluating it would run
    // the wrong thing entirely.
    const chain = steps([11, '440'], [10, '(440)'], [0, 'SinOsc.ar(440).play;']);

    assert.equal(pickBlock(chain), undefined);
});

test('nested blocks resolve to the outer one', () => {
    const chain = steps(
        [1, 'x'],
        [0, '(x)'],
        [0, '(\n(x)\n)'],
    );

    assert.equal(pickBlock(chain), 2);
});

test('a function body is not a block', () => {
    // `{ … }` is a step in the chain but is not parenthesised, so the `( … )`
    // around it still wins.
    const chain = steps(
        [2, 'freq'],
        [1, '{ |freq| freq }'],
        [0, '(\n{ |freq| freq }.value(440);\n)'],
    );

    assert.equal(pickBlock(chain), 2);
});

test('no parentheses at all means no block', () => {
    assert.equal(pickBlock(steps([0, 'SinOsc'], [0, 'SinOsc.ar;'])), undefined);
});
