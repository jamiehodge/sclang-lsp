#!/bin/sh
# Build a patched sclang that can dump its own parse trees.
#
# DumpParseNode.cpp has been in SuperCollider for years with no caller: no
# flag, no primitive, nothing. `expose-dumpparsenode.patch` adds an env-var
# hook in compileClass, so class-library compilation emits each file's parse
# tree between markers. That is the only oracle that can validate expression
# *structure*; the symbol oracle only sees declarations.
#
#   ./oracle/build-sclang-dump.sh [path-to-supercollider-clone]
#
# Then:
#   SCLANG_DUMP_PARSE=1 <build>/lang/sclang -a -l conf.yaml -i none quit.scd
#
# The build is sclang only — no Qt, no IDE, no scsynth — which keeps it to a
# few minutes.

set -e
here=$(cd "$(dirname "$0")" && pwd)
sc=${1:-$HOME/Developer/supercollider}

if [ ! -d "$sc" ]; then
    echo "==> cloning SuperCollider into $sc"
    git clone --recurse-submodules --shallow-submodules --depth 1 \
        https://github.com/supercollider/supercollider.git "$sc"
fi

echo "==> applying the dump patch"
cd "$sc"
git apply --check "$here/expose-dumpparsenode.patch" 2>/dev/null &&
    git apply "$here/expose-dumpparsenode.patch" ||
    echo "    (already applied, or needs rebasing onto a newer develop)"

echo "==> configuring (sclang only)"
cmake -B build -DCMAKE_BUILD_TYPE=Release \
    -DSC_IDE=OFF -DSC_QT=OFF -DSC_EL=OFF -DSC_VIM=OFF -DSUPERNOVA=OFF -DNO_X11=ON

echo "==> building"
cmake --build build --target sclang -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)"

echo "==> built $sc/build/lang/sclang"
