#!/usr/bin/env bash
# PGO training: build pdflatex.fmt and compile the benchmark corpus with the instrumented
# engine ($BIN). Called by tools/pgo.sh from its work directory.
set -u
CORPUS="$(cd "$(dirname "$0")/../../../bench/corpus" && pwd)"
cp "$CORPUS"/*.tex .
"$BIN" -ini -jobname=pdflatex -progname=pdflatex -translate-file=cp227.tcx -interaction=batchmode '*pdflatex.ini' >/dev/null 2>&1 || true
for f in 01-latex-hello 02-article 03-math 04-book 05-tikz 06-preamble; do
  for pass in 1 2; do
    "$BIN" -progname=pdflatex -fmt=pdflatex -interaction=batchmode "$f" >/dev/null 2>&1 || true
  done
done
