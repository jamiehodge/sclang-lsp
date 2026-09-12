// Checks the one place the two halves meet.
//
// `block.test.ts` asserts what `pickBlock` does with a chain of selection
// ranges. This asserts that the chains are the ones sclang-lsp really returns,
// by running the built server over stdio and feeding its answers through the
// same function. Without it the unit tests would be checking a guess.
//
//     node scripts/e2e-selection.js [path/to/sclang-lsp]

const { spawn } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');
const assert = require('assert');

const { pickBlock } = require('../out/block');

const repoRoot = path.resolve(__dirname, '..', '..', '..');
// Whichever build is newer, so this cannot quietly check a stale binary.
const server =
    process.argv[2] ??
    ['release', 'debug']
        .map((profile) => path.join(repoRoot, 'target', profile, 'sclang-lsp'))
        .filter(fs.existsSync)
        .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs)[0];

if (!server) {
    console.error('no sclang-lsp binary; run `cargo build` first');
    process.exit(1);
}

// An empty directory, so the scan finishes at once and nothing depends on what
// SuperCollider happens to be installed on this machine.
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'sclang-lsp-e2e-'));

const child = spawn(server, [], { stdio: ['pipe', 'pipe', 'inherit'] });

let buffer = Buffer.alloc(0);
const pending = new Map();

child.stdout.on('data', (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    for (;;) {
        const header = buffer.indexOf('\r\n\r\n');
        if (header < 0) return;

        const length = Number(/Content-Length: (\d+)/.exec(buffer.slice(0, header))[1]);
        const start = header + 4;
        if (buffer.length < start + length) return;

        const message = JSON.parse(buffer.slice(start, start + length).toString('utf8'));
        buffer = buffer.slice(start + length);

        const resolve = pending.get(message.id);
        if (resolve) {
            pending.delete(message.id);
            if (message.error) {
                console.error('server error:', JSON.stringify(message.error));
            }
            resolve(message.result);
        }
    }
});

let nextId = 0;

function send(message) {
    const body = Buffer.from(JSON.stringify(message), 'utf8');
    child.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
    child.stdin.write(body);
}

function request(method, params) {
    const id = (nextId += 1);
    return new Promise((resolve) => {
        pending.set(id, resolve);
        send({ jsonrpc: '2.0', id, method, params });
    });
}

function notify(method, params) {
    send({ jsonrpc: '2.0', method, params });
}

/** The chain at a position, innermost first, as `pickBlock` wants it. */
async function chainAt(uri, text, line, character) {
    const [chain] = await request('textDocument/selectionRange', {
        textDocument: { uri },
        positions: [{ line, character }],
    });

    const lines = text.split('\n');
    const offset = (position) =>
        lines.slice(0, position.line).reduce((n, l) => n + l.length + 1, 0) + position.character;

    const steps = [];
    for (let node = chain; node; node = node.parent) {
        steps.push({
            text: text.slice(offset(node.range.start), offset(node.range.end)),
            column: node.range.start.character,
        });
    }
    return steps;
}

async function main() {
    await request('initialize', {
        capabilities: {},
        initializationOptions: { classLibraryPaths: [dir] },
    });
    notify('initialized', {});

    const cases = [
        {
            name: 'block around the cursor',
            text: '(\n\tSinOsc.ar(440).play;\n)\n',
            at: [1, 12],
            expect: '(\n\tSinOsc.ar(440).play;\n)',
        },
        {
            name: 'a parenthesis in a string is not structure',
            text: '(\n\t"a ( b".postln; // ) no\n\tSinOsc.ar(440);\n)\n',
            at: [2, 12],
            expect: '(\n\t"a ( b".postln; // ) no\n\tSinOsc.ar(440);\n)',
        },
        {
            name: 'an argument list is not a block',
            text: 'SinOsc.ar(440).play;\n',
            at: [0, 11],
            expect: undefined,
        },
        {
            name: 'nested blocks resolve to the outer one',
            text: '(\n(\n\t1 + 2;\n);\n)\n',
            at: [2, 2],
            expect: '(\n(\n\t1 + 2;\n);\n)',
        },
    ];

    let failures = 0;

    for (const [index, testCase] of cases.entries()) {
        const uri = `file://${path.join(dir, `case${index}.scd`)}`;
        notify('textDocument/didOpen', {
            textDocument: { uri, languageId: 'supercollider', version: 1, text: testCase.text },
        });

        const steps = await chainAt(uri, testCase.text, ...testCase.at);
        const picked = pickBlock(steps);
        const actual = picked === undefined ? undefined : steps[picked].text;

        try {
            assert.deepEqual(actual, testCase.expect);
            console.log(`  ok   ${testCase.name}`);
        } catch {
            failures += 1;
            console.log(`  FAIL ${testCase.name}`);
            console.log(`       expected: ${JSON.stringify(testCase.expect)}`);
            console.log(`       actual:   ${JSON.stringify(actual)}`);
            console.log(`       chain:    ${JSON.stringify(steps)}`);
        }
    }

    await request('shutdown', null);
    notify('exit', null);
    fs.rmSync(dir, { recursive: true, force: true });

    console.log(failures === 0 ? '\nselection ranges agree with the server' : `\n${failures} failed`);
    process.exit(failures === 0 ? 0 : 1);
}

main();
