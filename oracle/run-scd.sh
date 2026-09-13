#!/bin/sh
# Diff script parsing against sclang's own compiler.
#
# Every other oracle here runs over the class library, which is `.sc` class
# files. Those never contain a top-level `( … )` block, a bare `var`, a literal
# collection at the start of a statement, or most of what `cmdlinecode` allows —
# so the syntax people actually type into a scratch buffer had no differential
# coverage at all.
#
# The corpus is the help files: thousands of runnable `code::` examples written
# by the people who designed the language, plus any `.scd` files installed. Each
# is handed to `String:compile`, which parses without running anything — the
# only reason this is safe, since evaluating the corpus would boot servers, open
# windows and make noise.
#
#   ./oracle/run-scd.sh [path-to-sclang]

set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
sclang=${1:-/Applications/SuperCollider.app/Contents/MacOS/sclang}
corpus="$here/scd-corpus"

if [ ! -x "$sclang" ]; then
    echo "sclang not found at $sclang" >&2
    echo "usage: $0 [path-to-sclang]" >&2
    exit 1
fi

resources=$(dirname "$(dirname "$sclang")")/Resources
support="$HOME/Library/Application Support/SuperCollider"

echo "==> building the corpus"
cd "$root"
cargo run --release --quiet --example scd_corpus -- "$corpus" \
    "$resources/HelpSource" "$support/Extensions" "$support/downloaded-quarks"

echo "==> asking sclang which of them compile"
SCLANG_SCD_CORPUS="$corpus" SCLANG_ORACLE_OUT="$here/oracle-scd.tsv" \
    "$sclang" -i none "$here/dump-scd.scd" 2>&1 |
    grep -E "^oracle:" || true

if [ ! -s "$here/oracle-scd.tsv" ]; then
    echo "oracle dump is empty; did sclang fail to compile the class library?" >&2
    exit 1
fi

echo "==> comparing"
cargo run --release --quiet --example scd_oracle -- "$here/oracle-scd.tsv" "$corpus"
