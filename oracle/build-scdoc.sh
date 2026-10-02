#!/bin/sh
# Fetch and build SuperCollider's own SCDoc parser as a test oracle.
#
# Upstream checks in the flex and bison output alongside SCDoc.l and SCDoc.y,
# and ships a standalone driver, main.cpp, that parses one help file and prints
# its tree. So the oracle is upstream's parser exactly, built with nothing but
# a C++ compiler: no flex, no bison, no sclang, and no other source from the
# tree. The crate is not linked against it; the two are diffed instead.
#
#   ./oracle/build-scdoc.sh [ref]        # default ref: Version-3.14.1
#
# The default is the release the crate is ported from. 3.14 only added tags
# (`subsubsection::`, `math::`), so an older ref still serves for a corpus that
# does not use them.

set -e
here=$(cd "$(dirname "$0")" && pwd)
ref=${1:-Version-3.14.1}
work="$here/scdoc"
raw="https://raw.githubusercontent.com/supercollider/supercollider/$ref/SCDoc"

command -v curl >/dev/null || { echo "needs curl" >&2; exit 1; }
command -v c++ >/dev/null || { echo "needs a C++ compiler" >&2; exit 1; }

mkdir -p "$work"
echo "==> fetching SCDoc @ $ref"
for f in SCDoc.h SCDoc.cpp SCDoc.tab.hpp SCDoc.tab.cpp lex.scdoc.cpp main.cpp; do
    curl -fsSL "$raw/$f" -o "$work/$f"
done

# The generated sources predate the compilers that build them now; their
# warnings are upstream's and say nothing about the comparison.
echo "==> building"
c++ -O2 -w -I"$work" \
    "$work/main.cpp" "$work/SCDoc.cpp" "$work/SCDoc.tab.cpp" "$work/lex.scdoc.cpp" \
    -o "$here/scdoc_dump"

echo "==> built $here/scdoc_dump"
