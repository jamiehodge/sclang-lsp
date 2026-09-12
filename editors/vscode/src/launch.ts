// How sclang is found and started.
//
// No editor API, so the test that spawns a real sclang can import exactly what
// the extension uses. That is the point of the split: a test with its own copy
// of the arguments would assert its own copy, and would not have noticed
// `-i vscode`.

import * as fs from 'fs';
import * as path from 'path';

/**
 * The IDE name sclang is started under.
 *
 * Load-bearing, and not for the reason the flag's name suggests. `-i` also
 * decides whether sclang starts its terminal reader: under any name but
 * `none` it assumes an IDE is driving it on some other channel and never reads
 * stdin at all, so every evaluation is discarded without a word. Evaluation
 * here *is* stdin. `none` is also the honest answer, since this is not the Qt
 * IDE and provides none of its primitives.
 */
export const IDE_NAME = 'none';

/** sclang prints this once the class library is in. */
export const READY = 'compile done';

export function sclangArgs(extra: readonly string[] = []): string[] {
    return ['-i', IDE_NAME, ...extra];
}

/**
 * An explicit path, then the usual place for the platform, then `PATH`.
 *
 * `configured` is passed in rather than read here, so this module stays
 * independent of the editor's settings.
 */
export function findSclang(configured = ''): string | undefined {
    const explicit = configured.trim();
    if (explicit) {
        return fs.existsSync(explicit) ? explicit : undefined;
    }

    for (const candidate of platformCandidates()) {
        if (fs.existsSync(candidate)) {
            return candidate;
        }
    }

    const exe = process.platform === 'win32' ? 'sclang.exe' : 'sclang';
    return (process.env.PATH ?? '')
        .split(path.delimiter)
        .filter(Boolean)
        .map((dir) => path.join(dir, exe))
        .find((candidate) => fs.existsSync(candidate));
}

function platformCandidates(): string[] {
    switch (process.platform) {
        case 'darwin':
            return ['/Applications/SuperCollider.app/Contents/MacOS/sclang'];
        case 'win32':
            // The install directory carries the version, so it has to be found
            // rather than named.
            return ['C:\\Program Files', 'C:\\Program Files (x86)'].flatMap((root) => {
                try {
                    return fs
                        .readdirSync(root)
                        .filter((entry) => entry.startsWith('SuperCollider'))
                        .map((entry) => path.join(root, entry, 'sclang.exe'));
                } catch {
                    return [];
                }
            });
        default:
            return ['/usr/bin/sclang', '/usr/local/bin/sclang'];
    }
}
