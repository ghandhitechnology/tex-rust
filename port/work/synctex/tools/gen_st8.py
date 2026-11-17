"""Generate the cases that \\input files by absolute path (so their .synctex files can be
byte-identical): cases/st8.tex, a long plain TeX document (many pages) so that the
.synctex.gz writer goes through several 8K gzprintf buffer cycles, and cases/가나.tex.
suite.py runs this first, because the paths depend on where the checkout lives."""
import pathlib
import random

CASES = pathlib.Path(__file__).resolve().parents[1] / "cases"
BS = "\\"


def abs_cases():
    c = str(CASES).replace("\\", "/")
    # lower-case drive letter: TeX Live's kpathsea reports absolute names that way
    return c[0].lower() + c[1:]


def write():
    c = abs_cases()
    rnd = random.Random(7)
    words = ("lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor "
             "incididunt ut labore et dolore magna aliqua AVAWAY office ff fi ffl").split()
    out = ["% big document for gz buffering", BS + "hsize=4in " + BS + "vsize=5in"]
    for i in range(400):
        line = " ".join(rnd.choice(words) for _ in range(rnd.randint(5, 25)))
        if i % 7 == 0:
            line += " $x_{%d}+y$" % i
        if i % 11 == 0:
            line += " " + BS + "hbox{box %d}" % i + BS + "kern%dpt" % (i % 5)
        if i % 13 == 0:
            line += " " + BS + "vrule width %dpt" % (i % 4 + 1)
        out.append(line)
        if i % 5 == 4:
            out.append("")
        if i % 97 == 0:
            out.append(BS + "input " + c + "/st2b")
    out.append(BS + "bye")
    (CASES / "st8.tex").write_text("\n".join(out) + "\n", encoding="utf-8", newline="\n")
    (CASES / "가나.tex").write_text("Hangul file name, absolute path " + BS + "hbox{x}." + BS + "par\n"
                                   + BS + "input " + c + "/café\n", encoding="utf-8", newline="\n")


if __name__ == "__main__":
    write()
