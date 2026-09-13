// The sclang process, its post window, and the pipe in both directions.
//
// Owning the child is the whole point. The post window is its stdout, the
// compile errors are in that same stream, and evaluation is its stdin — three
// features that all fall out of one decision, and none of which are reachable
// from outside the process.

import { ChildProcess, spawn } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as vscode from 'vscode';
import { findSclang, READY, sclangArgs } from './launch';

/** How long `0.exit` gets before the process is taken down by force. */
const QUIT_GRACE_MS = 3000;

/** How long to wait for the class library before giving up on the banner. */
const COMPILE_TIMEOUT_MS = 120_000;

export class Sclang implements vscode.Disposable {
    private child: ChildProcess | undefined;
    private ready: Promise<void> | undefined;

    private readonly post = vscode.window.createOutputChannel('SuperCollider');
    private readonly emitter = new vscode.EventEmitter<string>();

    /** Everything the child writes, chunked as it arrives. */
    readonly onOutput = this.emitter.event;

    /**
     * Where a multi-line region is staged before being loaded.
     *
     * Stable rather than unique, so a stack trace always names the same file
     * and the temp directory does not fill up over a session.
     */
    private readonly scratch = path.join(os.tmpdir(), `sclang-lsp-${process.pid}.scd`);

    get running(): boolean {
        return this.child !== undefined;
    }

    show(): void {
        this.post.show(true);
    }

    /** Start sclang, resolving when the class library has compiled. */
    async start(): Promise<void> {
        if (this.ready) {
            return this.ready;
        }

        const binary = findSclang(config().get<string>('sclang.path', ''));
        if (!binary) {
            void vscode.window
                .showErrorMessage('sclang-lsp: could not find sclang.', 'Open settings')
                .then((choice) => {
                    if (choice === 'Open settings') {
                        void vscode.commands.executeCommand(
                            'workbench.action.openSettings',
                            'sclang-lsp.sclang.path',
                        );
                    }
                });
            throw new Error('sclang not found');
        }

        // See `launch.ts` for why the IDE name is not negotiable.
        const extra = config().get<string[]>('sclang.args', []);
        const child = spawn(binary, sclangArgs(extra), {
            cwd: vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? os.homedir(),
            stdio: ['pipe', 'pipe', 'pipe'],
        });

        this.child = child;
        this.post.clear();
        this.post.show(true);

        child.stdout?.setEncoding('utf8');
        child.stderr?.setEncoding('utf8');
        child.stdout?.on('data', (chunk: string) => this.receive(chunk));
        child.stderr?.on('data', (chunk: string) => this.receive(chunk));

        // Writing to a pipe whose far end has gone raises on the stream
        // rather than at the call, and an unhandled one takes the extension
        // host with it. The exit handler below is what actually reports this.
        child.stdin?.on('error', () => undefined);

        // Last resort. `dispose` asks sclang to leave properly and waits, but
        // if the extension host is going down faster than that, a synchronous
        // kill on the way out beats leaving a process behind. An orphaned
        // sclang does not idle quietly: with its stdin gone it spins on EOF,
        // and a pegged core is audible as dropouts long before it is visible.
        const killOnHostExit = () => {
            try {
                child.kill('SIGKILL');
            } catch {
                // Already gone.
            }
        };
        process.once('exit', killOnHostExit);
        child.once('exit', () => process.removeListener('exit', killOnHostExit));

        // sclang can leave at any point — `0.exit` from the buffer, a crash, a
        // class library that fails to compile. Whenever it does, this object
        // has to stop believing it has a process, or the next evaluation is
        // written into a dead pipe and vanishes.
        child.on('exit', (code, signal) => {
            if (this.child !== child) {
                return;
            }
            this.child = undefined;
            this.ready = undefined;
            this.post.appendLine(`\n[sclang-lsp] sclang exited (${code ?? signal}).`);
        });

        this.ready = this.awaitCompile(child);
        return this.ready;
    }

    /**
     * Stop sclang, giving it the chance to leave quietly.
     *
     * `Main.shutdown` runs `Server.quitAll` on the way out, so an orderly exit
     * is what stops scsynth being orphaned on the audio device. Force is the
     * fallback, not the plan.
     */
    async stop(): Promise<void> {
        const child = this.child;
        if (!child) {
            return;
        }

        this.child = undefined;
        this.ready = undefined;

        const gone = new Promise<boolean>((resolve) => {
            const timer = setTimeout(() => resolve(false), QUIT_GRACE_MS);
            child.once('exit', () => {
                clearTimeout(timer);
                resolve(true);
            });
        });

        child.stdin?.write('0.exit;\n');
        if (!(await gone)) {
            this.post.appendLine('\n[sclang-lsp] sclang did not exit; killing it.');
            force(child);
        }
    }

    async restart(): Promise<void> {
        await this.stop();
        await this.start();
    }

    /**
     * Evaluate a region of SuperCollider.
     *
     * Over a pipe sclang reads a line at a time, so a multi-line region cannot
     * simply be written to stdin — it would be evaluated line by line and a
     * block would never close. Flattening it is worse still, because the first
     * `//` comment would swallow everything after it. So anything spanning
     * more than one line goes via a file, which keeps the text exactly as
     * written.
     *
     * That file is padded with the blank lines above the region, so the line
     * numbers sclang reports in an error are the ones in the buffer the user
     * is looking at.
     */
    async evaluate(code: string, options: { line?: number; label?: string } = {}): Promise<void> {
        await this.start();

        if (config().get<boolean>('echo', true)) {
            this.post.appendLine(`\n${options.label ?? code.trim()}`);
        }

        if (code.includes('\n')) {
            const padding = '\n'.repeat(options.line ?? 0);
            await fs.promises.writeFile(this.scratch, padding + code, 'utf8');
            this.write(`${quote(this.scratch)}.load;`);
        } else {
            this.write(code);
        }
    }

    private write(line: string): void {
        this.child?.stdin?.write(`${line}\n`);
    }

    private receive(chunk: string): void {
        this.post.append(chunk);
        this.emitter.fire(chunk);
    }

    /**
     * Resolve when the class library is in, so an evaluation triggered at
     * startup does not race the compile.
     */
    private awaitCompile(child: ChildProcess): Promise<void> {
        return new Promise((resolve, reject) => {
            const settle = (fn: () => void) => {
                clearTimeout(timer);
                listener.dispose();
                child.off('exit', onExit);
                fn();
            };

            const timer = setTimeout(
                () => settle(() => resolve()),
                COMPILE_TIMEOUT_MS,
            );

            const listener = this.onOutput((chunk) => {
                if (chunk.includes(READY)) {
                    settle(() => resolve());
                }
            });

            // A class library that fails to compile can take sclang down with
            // it, so waiting for a banner that will never arrive is not an
            // option. The exit itself is reported by the handler in `start`.
            const onExit = () => settle(() => reject(new Error('sclang exited during startup')));
            child.once('exit', onExit);
        });
    }

    dispose(): void {
        void this.shutdown();
    }

    /// Stop sclang and release everything, as a promise the caller can wait on.
    ///
    /// `dispose` cannot: the interface returns void, so VS Code has nothing to
    /// await and the extension host can exit first — which is how a process
    /// gets left behind. `deactivate` uses this instead.
    async shutdown(): Promise<void> {
        await this.stop();
        this.emitter.dispose();
        this.post.dispose();
        await fs.promises.rm(this.scratch, { force: true }).catch(() => undefined);
    }
}

function config(): vscode.WorkspaceConfiguration {
    return vscode.workspace.getConfiguration('sclang-lsp');
}

/** A SuperCollider string literal. */
function quote(text: string): string {
    return `"${text.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}

function force(child: ChildProcess): void {
    if (process.platform === 'win32' && child.pid !== undefined) {
        // Windows has no signal to send, and scsynth is a child of sclang, so
        // the tree has to go together.
        spawn('taskkill', ['/pid', String(child.pid), '/T', '/F']);
    } else {
        child.kill('SIGKILL');
    }
}
