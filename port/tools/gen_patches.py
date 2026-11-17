"""Post-translation patches of the generated code (engine/src/gen), applied after
tools/w2rs/emit.py. Each patch replaces one exact piece of generated text (asserted, so
a translator change cannot silently skip it) with a call to a hand-written runtime
routine that has the same effect, faster.

Usage: python -I tools/gen_patches.py
"""
import pathlib

GEN = pathlib.Path(__file__).resolve().parents[1] / "engine" / "src" / "gen"

PATCHES = [
    # load_fmt_file (web2c tex.ch): hash[hash_base+1..hash_top] := hash[hash_base] after
    # hash[hash_base] := (0, 0), i.e. zero-fill a freshly allocated array
    ("""            {
                let __end10: i32 = self.hashtop;
                x = 515;
                if (x as i32) <= __end10 {
                    loop {
                        (*ix(self.hash, x)) = (*ix(self.hash, 514));
                        let __c11 = (x as i32) < __end10;
                        x = ((x as i32) + 1) as i32;
                        if !__c11 { break; }
                    }
                }
            }""",
     """            // hash[hash_base+1..hash_top] := hash[hash_base] (tools/gen_patches.py)
            self.fill_fresh_from(self.hash, 514, 515, self.hashtop);
            x = if self.hashtop >= 515 { self.hashtop + 1 } else { 515 };"""),
    # load_fmt_file: eqtb[eqtb_size+1..eqtb_top] := eqtb[undefined_control_sequence]
    ("""            {
                let __end12: i32 = self.eqtbtop;
                x = 30193;
                if (x as i32) <= __end12 {
                    loop {
                        (*ix(self.eqtb, x)) = (*ix(self.eqtb, 26627));
                        let __c13 = (x as i32) < __end12;
                        x = ((x as i32) + 1) as i32;
                        if !__c13 { break; }
                    }
                }
            }""",
     """            // eqtb[eqtb_size+1..eqtb_top] := eqtb[undefined_control_sequence], lazily
            // (rt/fast.rs eqtb_lazy_from; tools/gen_patches.py)
            self.eqtb_lazy_from(26627, 30193, self.eqtbtop);
            x = if self.eqtbtop >= 30193 { self.eqtbtop + 1 } else { 30193 };"""),
    # INITEX initialize: eqtb[1..eqtb_top] := eqtb[undefined_control_sequence]; the
    # hash_extra part (beyond eqtb_size) is filled lazily, as in load_fmt_file
    ("""            {
                let __end41: i32 = self.eqtbtop;
                k = 1;
                if (k as i32) <= __end41 {
                    loop {
                        (*ix(self.eqtb, k)) = (*ix(self.eqtb, 26627));
                        let __c42 = (k as i32) < __end41;
                        k = ((k as i32) + 1) as i32;
                        if !__c42 { break; }
                    }
                }
            }""",
     """            // eqtb[1..eqtb_top] := eqtb[undefined_control_sequence] (tools/gen_patches.py):
            // up to eqtb_size now, the hash_extra part lazily (rt/fast.rs eqtb_lazy_from)
            self.fill_fresh_from(self.eqtb, 26627, 1, self.eqtbtop.min(30192));
            self.eqtb_lazy_from(26627, 30193, self.eqtbtop);
            k = if self.eqtbtop >= 1 { self.eqtbtop + 1 } else { 1 };"""),
    # id_lookup: a new control sequence in the hash_extra part gets its eqtb entry now
    ("""                                self.hashhigh += 1;
                                (*ix(self.hash, p)).set_lh((self.hashhigh + 30192));""",
     """                                self.hashhigh += 1;
                                self.eqtb_lazy_ensure(self.hashhigh + 30192);
                                (*ix(self.hash, p)).set_lh((self.hashhigh + 30192));"""),
    # pdf_new_obj: record the (virtual) offset of a new object for rt/pdf/parzip.rs
    ("""            (*ix(self.objtab, i)).int2 = (self.pdfgone + (self.pdfptr as i64));""",
     """            (*ix(self.objtab, i)).int2 = (self.pdfgone + (self.pdfptr as i64));
            self.parzip_note_offset(i);"""),
    # pdf finish: all deferred streams are written before the cross-reference section
    # (the only reader of object offsets) and the final byte count
    ("""                        self.pdfenddict();
                        if ((*ix(self.eqtb, 29376)).int() == 0) {
                            self.pdfprintinfo();
                        }
                        if self.pdfosenable {""",
     """                        self.pdfenddict();
                        if ((*ix(self.eqtb, 29376)).int() == 0) {
                            self.pdfprintinfo();
                        }
                        self.parzip_sync();
                        if self.pdfosenable {"""),
    # adv_char_width: round(s/10) for an integer s, exactly as zround of the real quotient
    # (an exact .5 quotient is representable; every other one is >= 0.1 from a boundary)
    ("            self.advcharwidths = zround(((s as f64) / (10 as f64)));",
     "            self.advcharwidths = crate::rt::fast::round_div10(s);"),
    # eq_define / geq_define / primitive: note meanings >= outer_call (rt/fast_macro.rs)
    ("""        (*ix(self.eqtb, p)).set_hh_b1((self.curlevel) as i16);
        (*ix(self.eqtb, p)).set_hh_b0((t) as i16);""",
     """        (*ix(self.eqtb, p)).set_hh_b1((self.curlevel) as i16);
        (*ix(self.eqtb, p)).set_hh_b0((t) as i16);
        self.note_eq_type(p, t as i32);"""),
    ("""        (*ix(self.eqtb, p)).set_hh_b1((1) as i16);
        (*ix(self.eqtb, p)).set_hh_b0((t) as i16);""",
     """        (*ix(self.eqtb, p)).set_hh_b1((1) as i16);
        (*ix(self.eqtb, p)).set_hh_b0((t) as i16);
        self.note_eq_type(p, t as i32);"""),
    ("""        (*ix(self.eqtb, self.curval)).set_hh_b0((c) as i16);""",
     """        (*ix(self.eqtb, self.curval)).set_hh_b0((c) as i16);
        self.note_eq_type(self.curval, c as i32);"""),
]


def main():
    files = {f: f.read_text(encoding="utf-8") for f in sorted(GEN.glob("*.rs"))}
    for old, new in PATCHES:
        hits = [f for f, s in files.items() if s.count(old) == 1]
        assert len(hits) == 1, f"patch target not found exactly once: {old[:120]!r}"
        files[hits[0]] = files[hits[0]].replace(old, new, 1)
    for f, s in files.items():
        f.write_text(s, encoding="utf-8", newline="\n")
    print(f"applied {len(PATCHES)} patches")


if __name__ == "__main__":
    main()
