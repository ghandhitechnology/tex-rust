#!/bin/sh
# difftest one SyncTeX case, then compare its .synctex files with cmpsync.py.
# Usage: sh tools/dt.sh NAME --files ... -- ARGS [-- ARGS2 ...]
cd "$(dirname "$0")/.." || exit 1
W="$(pwd -W 2>/dev/null || pwd)"
export TEXRS_WORK="$W/test"
if [ -d engine ]; then export TEXRS_ENGINE_DIR="$W/engine"; fi
python -I ../../tests/difftest.py "$@"
python -I tools/cmpsync.py "test/$1"
