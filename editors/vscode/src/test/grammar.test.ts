// The grammar's job is not colour. The server's semantic tokens do that, and
// they cover these same ranges and win wherever both apply. Its job is to tell
// VS Code where strings and comments are, because bracket matching,
// bracket-pair colouring and auto-closing read the TextMate token stream and
// semantic tokens do not feed that path. Without it `"("`, `$(`, `'('` and
// `// (` all pair with a later `)`.
//
// That property is invisible in the file itself — a wrong scope *name* still
// looks like a reasonable grammar — so it is asserted here against the same
// tokenizer VS Code uses.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as oniguruma from 'vscode-oniguruma';
import { INITIAL, parseRawGrammar, Registry, IGrammar } from 'vscode-textmate';

/**
 * How VS Code turns a scope name into a standard token type, and so what
 * decides whether the brackets inside a token count as structure.
 *
 * Taken from the regex VS Code 1.137 ships (`\b(comment|string|regex|regexp)`).
 * Only `comment` and `string` are reachable from this grammar, and a scope
 * that misses it — `constant.character`, say — leaves its brackets live.
 */
const SUPPRESSES_BRACKETS = /\b(comment|string|regex|regexp)\b/;

const GRAMMAR = path.join(__dirname, '..', '..', 'syntaxes', 'supercollider.tmLanguage.json');

let loaded: Promise<IGrammar> | undefined;

function grammar(): Promise<IGrammar> {
    loaded ??= (async () => {
        const wasm = fs.readFileSync(require.resolve('vscode-oniguruma/release/onig.wasm'));
        await oniguruma.loadWASM(wasm.buffer as ArrayBuffer);
        const registry = new Registry({
            onigLib: Promise.resolve({
                createOnigScanner: (sources) => new oniguruma.OnigScanner(sources),
                createOnigString: (source) => new oniguruma.OnigString(source),
            }),
            loadGrammar: async () => parseRawGrammar(fs.readFileSync(GRAMMAR, 'utf8'), GRAMMAR),
        });
        const found = await registry.loadGrammar('source.supercollider');
        assert.ok(found, 'the grammar failed to load');
        return found;
    })();
    return loaded;
}

/**
 * The scopes covering one character, tokenizing from the top of the file so
 * that a rule spanning lines — a block comment — is carried correctly.
 */
async function scopesAt(source: string, line: number, column: number): Promise<string[]> {
    const g = await grammar();
    let stack = INITIAL;
    const lines = source.split('\n');
    for (let i = 0; i < lines.length; i++) {
        const result = g.tokenizeLine(lines[i], stack);
        if (i === line) {
            const token = result.tokens.find((t) => t.startIndex <= column && column < t.endIndex);
            assert.ok(token, `no token at ${line}:${column} of ${JSON.stringify(source)}`);
            return token.scopes;
        }
        stack = result.ruleStack;
    }
    throw new Error(`line ${line} is past the end of ${JSON.stringify(source)}`);
}

/** Whether the character at this position is inert as far as brackets go. */
async function isInert(source: string, line: number, column: number): Promise<boolean> {
    const scopes = await scopesAt(source, line, column);
    return scopes.some((scope) => SUPPRESSES_BRACKETS.test(scope));
}

// The four cases ARCHITECTURE.md names as why the extension does not count
// parentheses itself. They are the same four that decide bracket matching.

test('a parenthesis in a double-quoted string is not a bracket', async () => {
    assert.ok(await isInert('a = "(";', 0, 5));
});

test('a parenthesis in a symbol is not a bracket', async () => {
    assert.ok(await isInert("a = '(';", 0, 5));
});

test('a parenthesis in a char literal is not a bracket', async () => {
    // `$(` is one token. This is the case that fails if the scope is named
    // `constant.character` rather than something matching `string`.
    assert.ok(await isInert('a = $(;', 0, 5));
});

test('a parenthesis in a line comment is not a bracket', async () => {
    assert.ok(await isInert('// (', 0, 3));
});

test('a parenthesis in a block comment is not a bracket', async () => {
    assert.ok(await isInert('/*\n(\n*/', 1, 0));
});

test('block comments nest, so an inner close does not end the outer one', async () => {
    // `/* /* */ (` — the `*/` closes only the comment the second `/*` opened,
    // which is what sclang does. Without nesting the `(` would be live code.
    assert.ok(await isInert('/* /* */ (\n*/', 0, 9));
});

test('an escaped quote does not end a string early', async () => {
    // `"a\"("` — if the `\"` were read as the closing quote, the `(` would be
    // outside the string.
    assert.ok(await isInert('a = "b\\"(";', 0, 8));
});

test('a char literal of a quote does not open a string', async () => {
    // `$"` is a character. A string rule winning here would swallow the rest
    // of the line and take the real string with it.
    const source = 'a = $" ++ "hi(";';
    assert.ok(await isInert(source, 0, 5), 'the $" itself');
    assert.ok(await isInert(source, 0, 13), 'the parenthesis in the real string after it');
});

// And the other half: the grammar must not make ordinary code inert, or
// bracket matching would stop working altogether.

test('a parenthesis in code is a bracket', async () => {
    assert.equal(await isInert('SinOsc.ar(440);', 0, 9), false);
});

test('a brace in code is a bracket', async () => {
    assert.equal(await isInert('Foo { bar { ^1 } }', 0, 4), false);
});

test('an argument pipe in code is a bracket', async () => {
    // `|` is declared a bracket pair in language-configuration.json, so it is
    // subject to the same rule.
    assert.equal(await isInert('{ |freq| freq }', 0, 2), false);
});

test('code after a comment ends is live again', async () => {
    assert.equal(await isInert('// note\nSinOsc.ar(440);', 1, 9), false);
});

test('code after a string closes is live again', async () => {
    assert.equal(await isInert('a = "x"; SinOsc.ar(440);', 0, 18), false);
});
