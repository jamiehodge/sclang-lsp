// The format here belongs to sclang, not to this project, so the fixture is
// real output — captured from sclang 3.13 compiling a class library with a
// deliberate syntax error in it — rather than something written to match the
// regexes.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CompileErrorParser } from '../compile-errors';

const REAL_OUTPUT = `compiling class library...
\tFound 855 primitives.
\tCompiling directory '/Applications/SuperCollider.app/Contents/Resources/SCClassLibrary'
\tCompiling directory '/tmp/badlib'
ERROR: syntax error, unexpected '}'
  in file '/tmp/badlib/BadClass.sc'
  line 4 char 2:

  \t}
   ^
  }
-----------------------------------
ERROR: file '/tmp/badlib/BadClass.sc' parse failed
error parsing
`;

test('finds the error, its file and its position', () => {
    const batch = new CompileErrorParser().feed(REAL_OUTPUT);

    assert.equal(batch.reset, true);
    assert.deepEqual(batch.errors, [
        {
            file: '/tmp/badlib/BadClass.sc',
            // sclang says line 4 char 2, counting from one.
            line: 3,
            character: 1,
            message: "syntax error, unexpected '}'",
        },
    ]);
});

test('the summary line that follows is not a second error', () => {
    // `ERROR: file '...' parse failed` names a file but gives no position, so
    // there is nowhere to put a marker and nothing to add.
    const batch = new CompileErrorParser().feed(REAL_OUTPUT);
    assert.equal(batch.errors.length, 1);
});

test('a report split across chunks is still found', () => {
    // The stream arrives in whatever pieces the pipe hands over, including
    // ones that split a line down the middle.
    const parser = new CompileErrorParser();
    const errors = [];
    for (let i = 0; i < REAL_OUTPUT.length; i += 7) {
        errors.push(...parser.feed(REAL_OUTPUT.slice(i, i + 7)).errors);
    }

    assert.equal(errors.length, 1);
    assert.equal(errors[0].file, '/tmp/badlib/BadClass.sc');
    assert.equal(errors[0].line, 3);
});

test('a new compile pass clears what the last one found', () => {
    const parser = new CompileErrorParser();
    parser.feed(REAL_OUTPUT);

    const second = parser.feed('compiling class library...\n\tFound 855 primitives.\n');
    assert.equal(second.reset, true);
    assert.deepEqual(second.errors, []);
});

test('ordinary output produces nothing', () => {
    const parser = new CompileErrorParser();
    const batch = parser.feed('sc3> -> a SinOsc\nhello\n-> nil\n');

    assert.equal(batch.reset, false);
    assert.deepEqual(batch.errors, []);
});

test('a runtime error with no file is not a compile error', () => {
    // Errors raised by running code report "in interpreted text", not a path,
    // and belong in the post window rather than in the problems panel.
    const parser = new CompileErrorParser();
    const batch = parser.feed(
        "ERROR: Parse error\n  in interpreted text\n  line 1 char 1:\n\n  )\n  ^\n",
    );

    assert.deepEqual(batch.errors, []);
});

test('a pass with several broken files reports each of them', () => {
    // The ordinary case after installing a quark that does not compile: one
    // report per file, each followed by the summary line and a rule. The
    // parser has to come back to a clean state between them.
    const parser = new CompileErrorParser();
    const batch = parser.feed(
        `compiling class library...
ERROR: syntax error, unexpected '}'
  in file '/tmp/a/A.sc'
  line 4 char 2:

  \t}
   ^
-----------------------------------
ERROR: syntax error, unexpected VAR, expecting '}'
  in file '/tmp/a/B.sc'
  line 9 char 3:

  \tvar x;
    ^
-----------------------------------
ERROR: file '/tmp/a/A.sc' parse failed
error parsing
`,
    );

    assert.deepEqual(
        batch.errors.map((e) => [e.file, e.line, e.character]),
        [
            ['/tmp/a/A.sc', 3, 1],
            ['/tmp/a/B.sc', 8, 2],
        ],
    );
});

test('two errors in one file are both kept', () => {
    const parser = new CompileErrorParser();
    const batch = parser.feed(
        `compiling class library...
ERROR: first problem
  in file '/tmp/a/A.sc'
  line 4 char 2:

-----------------------------------
ERROR: second problem
  in file '/tmp/a/A.sc'
  line 9 char 1:

`,
    );

    assert.deepEqual(
        batch.errors.map((e) => e.line),
        [3, 8],
    );
});
