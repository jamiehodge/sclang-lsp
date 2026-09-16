#!/bin/sh
# Diff method resolution against sclang's own dispatch.
#
# `Foo(...).bar` resolves to the method an instance of Foo would really call,
# found by walking Foo's superclass chain. This asks the locally installed
# sclang what *it* would select for the same pair — via
# `findRespondingMethodFor`, the resolution `Object` itself uses — and compares.
#
# That call is static and has no side effects, which is the only reason this
# oracle is possible: actually constructing one of every class would boot
# servers, open windows and throw.
#
#   ./oracle/run-resolution.sh [path-to-sclang]

set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
sclang=${1:-/Applications/SuperCollider.app/Contents/MacOS/sclang}

if [ ! -x "$sclang" ]; then
    echo "sclang not found at $sclang" >&2
    echo "usage: $0 [path-to-sclang]" >&2
    exit 1
fi

echo "==> asking sclang what dispatch selects"
SCLANG_ORACLE_OUT="$here/oracle-resolution.tsv" \
    "$sclang" -i none "$here/dump-resolution.scd" 2>&1 |
    grep -E "^oracle:|ERROR|error" || true

if [ ! -s "$here/oracle-resolution.tsv" ]; then
    echo "oracle dump is empty; did sclang fail to compile the class library?" >&2
    exit 1
fi

echo "==> comparing"
cd "$root"
# Extra arguments are class library roots. Not usually needed: the default now
# reads `sclang_conf.yaml`, so it sees the same directories sclang does,
# including a quark checked out somewhere of your own. Pass them to compare
# against a set this machine is not configured for.
shift 2>/dev/null || true
cargo run --release --quiet --example resolve_oracle -- "$here/oracle-resolution.tsv" "$@"
