// A thin client. The server does the work; this file finds the binary, starts
// it on stdio, and gets out of the way.
//
// Deliberately no evaluation, no post window, no server control. Those need a
// live sclang and are the editor's job rather than the language server's, so
// they do not belong behind an LSP connection — see ARCHITECTURE.md.

import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    context.subscriptions.push(
        vscode.commands.registerCommand('sclang-lsp.restart', () => restart(context)),
        vscode.commands.registerCommand('sclang-lsp.showLog', () => client?.outputChannel.show()),
        vscode.workspace.onDidChangeConfiguration(async (event) => {
            // Every setting here feeds the server's startup, so none of them
            // can take effect without a restart.
            if (event.affectsConfiguration('sclang-lsp')) {
                await restart(context);
            }
        }),
    );

    await start(context);
}

export function deactivate(): Promise<void> | undefined {
    return client?.stop();
}

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
        documentSelector: [{ scheme: 'file', language: 'supercollider' }],
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

function initializationOptions(): Record<string, unknown> {
    const paths = vscode.workspace
        .getConfiguration('sclang-lsp')
        .get<string[]>('classLibraryPaths', []);

    // Send the key only when it has content. The server treats its presence as
    // an instruction to index exactly these directories, so passing an empty
    // array would mean "index nothing" rather than "use the defaults".
    return paths.length > 0 ? { classLibraryPaths: paths } : {};
}

interface Resolved {
    command?: string;
    problem?: string;
}

/**
 * Find the server binary: an explicit setting, then a cargo build in the
 * checkout this extension lives in, then PATH.
 *
 * The middle case is what makes `F5` work with no configuration while
 * developing the server, which is the common case for now.
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
            'could not find the sclang-lsp binary. Run `cargo build --release` in the repository, ' +
            'or set sclang-lsp.server.path.',
    };
}
