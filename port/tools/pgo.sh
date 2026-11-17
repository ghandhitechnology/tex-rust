#!/usr/bin/env bash
# Profile-guided optimisation build of the engine.
#   1. build an instrumented engine (target-pgo-gen)
#   2. run the training workloads in tests/pgo/ (format builds, plain and LaTeX documents)
#   3. merge the profiles and build the final engine into target-pgo/release/pdftex.exe
# Usage: bash port/tools/pgo.sh
set -euo pipefail
PORT="$(cd "$(dirname "$0")/.." && pwd)"
ENGINE="$PORT/engine"
PROF="$PORT/target-pgo-data"
PROFDATA="$(ls ~/.rustup/toolchains/stable-x86_64-pc-windows-msvc/lib/rustlib/x86_64-pc-windows-msvc/bin/llvm-profdata.exe)"
# TEXRS_TEXLIVE: the TeX Live 2026 installation (default: tools/texlive/2026 next to port/)
TL="$(cygpath -m "${TEXRS_TEXLIVE:-$PORT/../tools/texlive/2026}")"
TL="$(cd "$TL" && pwd -W)"
export TEXRS_SELFAUTOLOC="$(echo "${TL:0:1}" | tr A-Z a-z)${TL:1}/bin/windows"
export SOURCE_DATE_EPOCH=1700000000 FORCE_SOURCE_DATE=1

rm -rf "$PROF" && mkdir -p "$PROF"
PROFW="$(cygpath -m "$PROF")"
(cd "$ENGINE" && CARGO_TARGET_DIR=target-pgo-gen RUSTFLAGS="-Cprofile-generate=$PROFW" cargo build --release --features crt-exit)
BIN="$ENGINE/target-pgo-gen/release/pdftex.exe"

W="$PORT/test/_pgo"
rm -rf "$W" && mkdir -p "$W"
cp "$PORT/tests/cases/bigplain.tex" "$PORT/tests/cases/plain1.tex" "$W/"
cd "$W"
"$BIN" -ini -interaction=batchmode -translate-file=cp227.tcx plain '\dump' >/dev/null
for f in bigplain plain1; do "$BIN" -fmt=plain -interaction=batchmode -output-format=dvi "$f" >/dev/null || true; done
for extra in "$PORT"/tests/pgo/*.sh; do if [ -f "$extra" ]; then BIN="$BIN" bash "$extra"; fi; done

"$PROFDATA" merge -o "$PROF/merged.profdata" "$PROF"
(cd "$ENGINE" && CARGO_TARGET_DIR=target-pgo RUSTFLAGS="-Cprofile-use=$(cygpath -m "$PROF/merged.profdata")" cargo build --release)
echo "PGO engine: $ENGINE/target-pgo/release/pdftex.exe"
