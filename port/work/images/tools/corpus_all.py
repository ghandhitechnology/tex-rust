"""Run every PDF of cases/corpus_all.txt (page 1 and the last page) through
cases.run with \pdfinclusioncopyfonts=0 and \pdfobjcompresslevel=2."""
import pathlib
import sys
sys.path.insert(0, str(pathlib.Path(__file__).parent))
import cases

files = [cases.TEXMF / l for l in (cases.CASES / "corpus_all.txt").read_text().split("\n") if l.strip()]
start = int(sys.argv[1]) if len(sys.argv) > 1 else 0
ok = tot = 0
bad = []
for k, f in enumerate(files[start:], start):
    n = f.name
    body = (r"\pdfximage{%s}\hbox{\pdfrefximage\pdflastximage}" % n
            + r"\ifnum\pdflastximagepages>1 \pdfximage page \pdflastximagepages{%s}\hbox{\pdfrefximage\pdflastximage}\fi" % n)
    tex = cases.doc(r"\pdfminorversion=7 \pdfobjcompresslevel=2", body)
    tot += 1
    r = cases.run(f"all{k:03d}", tex, [f])
    ok += r
    if not r:
        bad.append((k, f.relative_to(cases.TEXMF).as_posix()))
print(f"{ok}/{tot} identical")
for b in bad:
    print("BAD", b)
