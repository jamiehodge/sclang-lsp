#!/bin/sh
# Regenerate the oracle and diff the parser against it.
#
# The class library is open-ended: anyone can drop classes into Extensions or
# install quarks, so there is no fixed set to check against and no useful
# fixture to commit. This script asks the *locally installed* sclang what it
# actually compiled, and the differ takes its file list from that answer — so
# the comparison automatically covers whatever classes exist on this machine,
# including ones written after this tool was.
#
# Re-run it after installing a quark, editing a class, or upgrading
# SuperCollider.
#
#   ./oracle/run.sh [path-to-sclang]

set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
sclang=${1:-/Applications/SuperCollider.app/Contents/MacOS/sclang}

if [ ! -x "$sclang" ]; then
    echo "sclang not found at $sclang" >&2
    echo "usage: $0 [path-to-sclang]" >&2
    exit 1
fi

echo "==> asking sclang what it compiled"
SCLANG_ORACLE_OUT="$here/oracle-symbols.tsv" \
    "$sclang" -i none "$here/dump-symbols.scd" 2>&1 |
    grep -E "^oracle:|ERROR|error" || true

if [ ! -s "$here/oracle-symbols.tsv" ]; then
    echo "oracle dump is empty; did sclang fail to compile the class library?" >&2
    exit 1
fi

echo "==> comparing"
cd "$root"
cargo run --release --quiet --example oracle -- "$here/oracle-symbols.tsv"
