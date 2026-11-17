#!/usr/bin/env bash
# Regenerate port/web from the TeX Live 2026 release sources:
#   tie merges the change files TeX Live's build applies to pdftex.web
#   (texk/web2c/pdftexdir/am/pdftex.am, pdftex_ch_srcs) into pdftex-final.ch,
#   then TeX Live's tangle writes pdftex.p (Pascal) and pdftex.pool (string pool).
# Usage: bash port/tools/gen-pascal.sh [OUTDIR]          (default OUTDIR: port/web)
#   TEXRS_WEB2C:   texk/web2c of a texlive-source checkout at branch2026
#                  (default: refs/texlive-source/texk/web2c next to port/)
#   TEXRS_TEXLIVE: TeX Live 2026, for tie and tangle (default: tools/texlive/2026)
set -euo pipefail
PORT="$(cd "$(dirname "$0")/.." && pwd)"
W="${TEXRS_WEB2C:-$PORT/../refs/texlive-source/texk/web2c}"
TL="${TEXRS_TEXLIVE:-$PORT/../tools/texlive/2026}"
G="${1:-$PORT/web}"
mkdir -p "$G"
G="$(cd "$G" && pwd)"
export PATH="$TL/bin/windows:$PATH"
CH="pdftexdir/tex.ch0 tex.ch tracingstacklevels.ch partoken-102.ch partoken.ch
locnull-optimize.ch showstream.ch zlib-fmt.ch enctexdir/enctex1.ch
enctexdir/enctex-pdftex.ch enctexdir/enctex2.ch unbalanced-braces.ch
synctexdir/synctex-def.ch0 synctexdir/synctex-mem.ch0 synctexdir/synctex-e-mem.ch0
synctexdir/synctex-e-mem.ch1 synctexdir/synctex-rec.ch0 synctexdir/synctex-rec.ch1
synctexdir/synctex-e-rec.ch0 synctexdir/synctex-pdf-rec.ch2 pdftexdir/pdftex.ch
pdftexdir/char-warning-pdftex.ch tex-binpool.ch"
cd "$W"
# shellcheck disable=SC2086
tie -c "$G/pdftex-final.ch" pdftexdir/pdftex.web $CH >/dev/null
cp pdftexdir/pdftex.web "$G/"
cd "$G"
tangle pdftex.web pdftex-final.ch
wc -l pdftex.p pdftex.pool
