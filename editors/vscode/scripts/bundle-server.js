// Copy the built server into the extension so the packaged VSIX carries it.
//
// An installed extension lives in ~/.vscode/extensions with no repository
// nearby, so without this it has no way to find the binary and the user has to
// set a path by hand. The copy makes the VSIX platform specific, which is what
// the per-target release artifacts are for.
//
//     node scripts/bundle-server.js [directory containing the binary]
//
// The argument is for cross compiling, where the build lands in
// target/<triple>/release rather than target/release. Without it, the host
// build is used.

const fs = require('fs');
const path = require('path');

const exe = process.platform === 'win32' ? 'sclang-lsp.exe' : 'sclang-lsp';
const repoRoot = path.resolve(__dirname, '..', '..', '..');
const from = process.argv[2] ?? path.join(repoRoot, 'target', 'release');
const source = path.join(from, exe);
const destDir = path.join(__dirname, '..', 'server');
const dest = path.join(destDir, exe);

if (!fs.existsSync(source)) {
    console.error(`no server binary at ${source}\nRun \`cargo build --release\` first.`);
    process.exit(1);
}

fs.mkdirSync(destDir, { recursive: true });
fs.copyFileSync(source, dest);
fs.chmodSync(dest, 0o755);
console.log(`bundled ${dest} (${(fs.statSync(dest).size / 1e6).toFixed(1)} MB)`);
