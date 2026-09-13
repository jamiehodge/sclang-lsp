// The extension: a language client, and an sclang.
//
// The two halves are independent and stay that way. The client finds the
// server binary, starts it on stdio and gets out of the way; the server never
// spawns sclang, never talks to one, and does not care whether one exists.
// Everything below that needs a live sclang goes through `Sclang`, which owns
// a child process of its own.
//
// They meet at exactly one point: deciding what to evaluate is a parsing
// question, so `region.ts` asks the server for it.

import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';
import { CompileErrors } from './diagnostics';
import { regionAt } from './region';
import { Sclang } from './sclang';

let client: LanguageClient | undefined;
let sclang: Sclang | undefined;

/** How long the evaluated region stays highlighted. */
const FLASH_MS = 250;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    context.subscriptions.push(
        vscode.commands.registerCommand('sclang-lsp.restart', () => restart(context)),
        vscode.commands.registerCommand('sclang-lsp.showLog', () => client?.outputChannel.show()),
        vscode.workspace.onDidChangeConfiguration(async (event) => {
            // Only the server's own settings feed its startup; the sclang ones
            // are read fresh each time it is launched.
            if (
                event.affectsConfiguration('sclang-lsp.server') ||
                event.affectsConfiguration('sclang-lsp.classLibraryPaths')
            ) {
                await restart(context);
            }
        }),
    );

    registerSclang(context);
    await start(context);
}

// ---- the language client ---------------------------------------------

async function start(context: vscode.ExtensionContext): Promise<void> {
    const server = resolveServer(context);
    if (!server.command) {
        void vscode.window.showErrorMessage(`sclang-lsp: ${server.problem}`, 'Open settings').then((choice) => {
            if (choice === 'Open settings') {
                void vscode.commands.executeCommand('workbench.action.openSettings', 'sclang-lsp.server.path');
            }
        });
        return;
    }

    const serverOptions: ServerOptions = {
        command: server.command,
        transport: TransportKind.stdio,
    };

    const clientOptions: LanguageClientOptions = {
        documentSelector: [
            { scheme: 'file', language: 'supercollider' },
            // A buffer that has never been saved is `untitled`, not `file`.
            // Leaving it out means the client never attaches to a scratch
            // buffer at all — no diagnostics, no completion, nothing — which
            // is exactly where SuperCollider tends to get written.
            { scheme: 'untitled', language: 'supercollider' },
        ],
        initializationOptions: initializationOptions(),
    };

    client = new LanguageClient('sclang-lsp', 'SuperCollider Language Server', serverOptions, clientOptions);
    await client.start();
}

async function restart(context: vscode.ExtensionContext): Promise<void> {
    await client?.stop();
    client = undefined;
    await start(context);
}

/// Shut both children down, and wait for them.
///
/// VS Code awaits this, which is the only chance to stop sclang properly —
/// `dispose` returns void, so nothing waits for it and the host can exit first.
/// A sclang left behind does not idle: with its stdin gone it spins on EOF, and
/// a pegged core shows up as audio dropouts before anything else.
export async function deactivate(): Promise<void> {
    await Promise.all([client?.stop(), sclang?.shutdown()]);
    client = undefined;
    sclang = undefined;
}

function initializationOptions(): Record<string, unknown> {
    const paths = vscode.workspace
        .getConfiguration('sclang-lsp')
        .get<string[]>('classLibraryPaths', []);

    // Send the key only when it has content. The server treats its presence as
    // an instruction to index exactly these directories, so passing an empty
    // array would mean "index nothing" rather than "use the defaults".
    return paths.length > 0 ? { classLibraryPaths: paths } : {};
}

// ---- running code ----------------------------------------------------

function registerSclang(context: vscode.ExtensionContext): void {
    // Module scope, so `deactivate` can wait for it.
    const owned = new Sclang();
    sclang = owned;
    const errors = new CompileErrors();
    const flash = vscode.window.createTextEditorDecorationType({
        backgroundColor: new vscode.ThemeColor('editor.findMatchHighlightBackground'),
    });

    context.subscriptions.push(
        owned,
        errors,
        flash,

        owned.onOutput((chunk) => {
            if (vscode.workspace.getConfiguration('sclang-lsp').get<boolean>('compileErrors', true)) {
                errors.feed(chunk);
            }
        }),

        vscode.commands.registerCommand('sclang-lsp.startSclang', () => guard(owned.start())),
        vscode.commands.registerCommand('sclang-lsp.stopSclang', () => guard(owned.stop())),
        vscode.commands.registerCommand('sclang-lsp.restartSclang', () => guard(owned.restart())),
        vscode.commands.registerCommand('sclang-lsp.showPost', () => owned.show()),

        // One command taking the code to run, so a user can bind whatever they
        // want — boot, quit, recompile — without the extension having an
        // opinion about which of those deserve to be commands of their own.
        vscode.commands.registerCommand('sclang-lsp.eval', (code: unknown) => {
            if (typeof code !== 'string' || code.trim().length === 0) {
                void vscode.window.showErrorMessage(
                    'sclang-lsp.eval needs the code to run, passed as the keybinding\'s "args".',
                );
                return;
            }
            guard(owned.evaluate(code));
        }),

        vscode.commands.registerCommand('sclang-lsp.evaluate', async () => {
            const editor = vscode.window.activeTextEditor;
            if (!editor) {
                return;
            }

            const region = await regionAt(editor);
            if (!region) {
                return;
            }

            editor.setDecorations(flash, [region.range]);
            setTimeout(() => editor.setDecorations(flash, []), FLASH_MS);

            await guard(
                owned.evaluate(region.text, {
                    line: region.range.start.line,
                    label: summarise(region.text),
                }),
            );
        }),
    );
}

/**
 * A failure to start sclang is already reported where it happens, and an
 * unhandled rejection here would surface as a second, less useful message.
 */
async function guard(work: Promise<void>): Promise<void> {
    try {
        await work;
    } catch {
        // Reported at the source.
    }
}

/** What to echo into the post window for a multi-line region. */
function summarise(code: string): string {
    const lines = code.split('\n');
    return lines.length > 1 ? `${lines[0].trim()} … (${lines.length} lines)` : code.trim();
}

// ---- finding the server ----------------------------------------------

interface Resolved {
    command?: string;
    problem?: string;
}

/**
 * Find the server binary, in order: an explicit setting, a copy packaged
 * inside this extension, a cargo build in the checkout it lives in, then PATH.
 *
 * The packaged copy is what makes an installed extension work, since an
 * installed one sits in ~/.vscode/extensions and has no repository near it.
 * The cargo build is what makes `F5` work with nothing configured while
 * developing the server.
 */
function resolveServer(context: vscode.ExtensionContext): Resolved {
    const exe = process.platform === 'win32' ? 'sclang-lsp.exe' : 'sclang-lsp';

    const configured = vscode.workspace
        .getConfiguration('sclang-lsp')
        .get<string>('server.path', '')
        .trim();

    if (configured) {
        return fs.existsSync(configured)
            ? { command: configured }
            : { problem: `no binary at the configured path: ${configured}` };
    }

    const bundled = path.join(context.extensionPath, 'server', exe);
    if (fs.existsSync(bundled)) {
        return { command: bundled };
    }

    // editors/vscode -> repo root
    const repoRoot = path.resolve(context.extensionPath, '..', '..');
    for (const profile of ['release', 'debug']) {
        const candidate = path.join(repoRoot, 'target', profile, exe);
        if (fs.existsSync(candidate)) {
            return { command: candidate };
        }
    }

    const onPath = (process.env.PATH ?? '')
        .split(path.delimiter)
        .filter(Boolean)
        .map((dir) => path.join(dir, exe))
        .find((candidate) => fs.existsSync(candidate));

    if (onPath) {
        return { command: onPath };
    }

    return {
        problem:
            'could not find the sclang-lsp binary. Run `cargo build --release` in the ' +
            'repository, or set sclang-lsp.server.path.',
    };
}
