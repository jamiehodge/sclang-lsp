#!/bin/sh
# Fetch and build SuperCollider's own lexer as a test oracle.
#
# Upstream extracted their lexer into `langutils/sc_lexer` — a standalone
# library whose only dependency is utf8proc. We do NOT link it: this crate
# stays pure Rust so it cross-compiles without a C++ toolchain. Instead we
# build it here and diff token streams against it in CI, which buys fidelity
# without the build dependency.
#
# Only the I/O differs from upstream's own driver (dump_file.cpp reads a file
# rather than taking source as argv, which cannot handle a class-library file);
# the lexing is entirely theirs.
#
#   ./oracle/build-sc-lexer.sh [ref]        # default ref: develop

set -e
here=$(cd "$(dirname "$0")" && pwd)
ref=${1:-develop}
work="$here/sc_lexer"
api="repos/supercollider/supercollider/contents/langutils/sc_lexer"

command -v gh >/dev/null || { echo "needs the gh CLI" >&2; exit 1; }
command -v c++ >/dev/null || { echo "needs a C++17 compiler" >&2; exit 1; }

mkdir -p "$work/include" "$work/src"
echo "==> fetching sc_lexer @ $ref"
for f in include/codepoint.hpp include/codepoint_stream.hpp include/lexer.hpp \
         include/normalise_source.hpp include/source_utils.hpp \
         include/text_location.hpp include/tokens.hpp \
         src/codepoint.cpp src/codepoint_stream.cpp src/source_utils.cpp \
         src/text_location.cpp src/tokens.cpp; do
    gh api "$api/$f?ref=$ref" --jq .content | base64 -d > "$work/$f"
done

# utf8proc: homebrew on macOS, system package elsewhere.
inc=""; lib=""
for prefix in /opt/homebrew /usr/local /usr; do
    if [ -f "$prefix/include/utf8proc.h" ]; then
        inc="-I$prefix/include"; lib="-L$prefix/lib"; break
    fi
done
[ -n "$inc" ] || { echo "utf8proc not found (brew install utf8proc)" >&2; exit 1; }

echo "==> building"
c++ -std=c++17 -O2 -I"$work/include" $inc \
    "$work"/src/*.cpp "$here/sc_lexer/dump_file.cpp" \
    $lib -lutf8proc -o "$here/sc_lexer_dump"

echo "==> built $here/sc_lexer_dump"
