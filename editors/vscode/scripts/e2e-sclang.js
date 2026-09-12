// Spawns a real sclang and checks that it answers.
//
// This exists because of a bug it would have caught. The extension shipped
// with `-i vscode`, and `-i` decides whether sclang starts its terminal reader
// at all: under any name but `none` it reads no stdin, so every evaluation was
// discarded in silence — no error, no prompt, nothing in the post window but
// the extension's own echo. Nothing in the unit tests could see that, because
// the failure is entirely on the far side of the pipe.
//
// So the arguments come from `launch.js`, the same module the extension uses.
// A test holding its own copy of them would assert its own copy and would have
// passed throughout.
//
// Skips when SuperCollider is not installed, which is why it is not part of
// `npm test`.
//
//     node scripts/e2e-sclang.js

const { spawn } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

const { findSclang, sclangArgs, READY, IDE_NAME } = require('../out/launch');

/** The class library, on a cold or loaded machine. */
const COMPILE_TIMEOUT_MS = 90_000;

/** One evaluation, once the library is in. */
const REPLY_TIMEOUT_MS = 20_000;

const binary = findSclang();
if (!binary) {
    console.log('skipped: no sclang on this machine');
    process.exit(0);
}

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'sclang-e2e-'));
const child = spawn(binary, sclangArgs(), { cwd: dir, stdio: ['pipe', 'pipe', 'pipe'] });

let output = '';
const waiters = [];

function receive(chunk) {
    output += chunk;
    for (const waiter of [...waiters]) {
        if (output.includes(waiter.needle)) {
            waiters.splice(waiters.indexOf(waiter), 1);
            clearTimeout(waiter.timer);
            waiter.resolve(true);
        }
    }
}

child.stdout.setEncoding('utf8');
child.stderr.setEncoding('utf8');
child.stdout.on('data', receive);
child.stderr.on('data', receive);

/** Resolve once `needle` has appeared anywhere in the output. */
function until(needle, ms) {
    if (output.includes(needle)) {
        return Promise.resolve(true);
    }
    return new Promise((resolve) => {
        const waiter = { needle, resolve };
        waiter.timer = setTimeout(() => {
            waiters.splice(waiters.indexOf(waiter), 1);
            resolve(false);
        }, ms);
        waiters.push(waiter);
    });
}

let failures = 0;

function check(name, ok, detail) {
    if (ok) {
        console.log(`  ok   ${name}`);
        return;
    }
    failures += 1;
    console.log(`  FAIL ${name}`);
    if (detail) {
        console.log(`       ${detail}`);
    }
}

function tail() {
    return `last output:\n${output.slice(-600).replace(/^/gm, '       | ')}`;
}

async function main() {
    console.log(`sclang: ${binary}\nargs:   ${sclangArgs().join(' ')}\n`);

    check(
        `started under the IDE name "${IDE_NAME}"`,
        IDE_NAME === 'none',
        `"${IDE_NAME}" means sclang never reads stdin, so nothing can be evaluated`,
    );

    const compiled = await until(READY, COMPILE_TIMEOUT_MS);
    check('class library compiles', compiled, compiled ? '' : tail());
    if (!compiled) {
        return finish();
    }

    // The regression itself: a line written to stdin comes back. A prompt is
    // not enough — `-i vscode` still printed the banner.
    child.stdin.write('"SCLANG-IS-LISTENING".postln;\n');
    check(
        'a single line written to stdin is evaluated',
        await until('-> SCLANG-IS-LISTENING', REPLY_TIMEOUT_MS),
        tail(),
    );

    // The other path `evaluate` takes. A multi-line region cannot go down the
    // pipe as-is — sclang reads a line at a time and the block would never
    // close — so it is staged in a file and loaded.
    const region = path.join(dir, 'region.scd');
    fs.writeFileSync(region, '(\nvar a = 40; // a comment\nvar b = 2;\n(a + b).postln;\n)\n');
    child.stdin.write(`"${region}".load;\n`);
    check(
        'a multi-line region loaded from a file is evaluated',
        await until('-> 42', REPLY_TIMEOUT_MS),
        tail(),
    );

    // What `stop` sends. `Main.shutdown` runs `Server.quitAll` on the way out,
    // which is what keeps scsynth off the audio device.
    child.stdin.write('0.exit;\n');
    check('exits on `0.exit`', await exited(REPLY_TIMEOUT_MS), 'still running');

    finish();
}

function exited(ms) {
    if (child.exitCode !== null) {
        return Promise.resolve(true);
    }
    return new Promise((resolve) => {
        const timer = setTimeout(() => resolve(false), ms);
        child.once('exit', () => {
            clearTimeout(timer);
            resolve(true);
        });
    });
}

function finish() {
    if (child.exitCode === null) {
        child.kill('SIGKILL');
    }
    fs.rmSync(dir, { recursive: true, force: true });
    console.log(failures === 0 ? '\nsclang answers' : `\n${failures} failed`);
    process.exit(failures === 0 ? 0 : 1);
}

main();
