# TeX Live pdftex vs Rust port (ms; 20 runs per engine and job, in alternating rounds)

Rust binary: `port\engine\target\release\pdftex.exe`

| job | identical output | TeX Live median (min) | Rust median (min) | speed-up (median) | speed-up (min) |
|---|---|---:|---:|---:|---:|
| plain format build (-ini) | yes | 114.8 (110.4) | 25.0 (23.7) | 4.59x | 4.65x |
| pdflatex format build (-ini) | yes | 4425.7 (4326.3) | 3700.1 (3655.4) | 1.20x | 1.18x |
| plain 70 pages, DVI | yes | 107.3 (102.6) | 25.0 (24.1) | 4.29x | 4.25x |
| plain 70 pages, PDF | yes | 145.2 (143.0) | 31.5 (30.3) | 4.61x | 4.72x |
| plain 700 pages, PDF | yes | 497.1 (486.1) | 184.4 (173.0) | 2.70x | 2.81x |
| LaTeX 01-latex-hello | yes | 136.9 (133.2) | 28.6 (26.1) | 4.78x | 5.11x |
| LaTeX 02-article | yes | 194.8 (189.7) | 60.3 (55.9) | 3.23x | 3.39x |
| LaTeX 03-math | yes | 262.4 (255.7) | 111.4 (106.3) | 2.36x | 2.41x |
| LaTeX 04-book | yes | 528.4 (500.1) | 345.2 (295.6) | 1.53x | 1.69x |
| LaTeX 05-tikz | yes | 1901.4 (1835.2) | 1364.7 (1328.1) | 1.39x | 1.38x |
| LaTeX 06-preamble | yes | 809.8 (799.2) | 413.9 (405.2) | 1.96x | 1.97x |
| LaTeX graphics | yes | 173.9 (169.5) | 44.3 (42.1) | 3.93x | 4.02x |
| LaTeX hyperref | yes | 263.2 (259.3) | 103.3 (99.7) | 2.55x | 2.60x |
| LaTeX microtype | yes | 227.2 (222.3) | 83.6 (80.1) | 2.72x | 2.78x |
| LaTeX 01-latex-hello, no fmt cache | yes | 133.9 (131.5) | 49.1 (46.3) | 2.73x | 2.84x |
| LaTeX 04-book, no fmt cache | yes | 511.6 (494.5) | 320.2 (314.9) | 1.60x | 1.57x |
| LaTeX 04-book, TL's installed format | yes | 517.3 (497.1) | 307.5 (299.2) | 1.68x | 1.66x |
