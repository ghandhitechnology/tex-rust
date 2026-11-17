//! Lexer and Parser edge cases (expected values follow xpdf's Lexer.cc
//! and Parser.cc).

use std::rc::Rc;

use pdfread::{set_err_quiet, Lexer, Object, Parser};

fn lex_all(s: &[u8]) -> Vec<Object> {
    set_err_quiet(true);
    let mut lx = Lexer::new(Rc::from(s), 0, None);
    let mut v = Vec::new();
    loop {
        let o = lx.get_obj();
        if o.is_eof() {
            break;
        }
        v.push(o);
    }
    v
}

fn parse(s: &[u8]) -> Object {
    set_err_quiet(true);
    let lx = Lexer::new(Rc::from(s), 0, None);
    let mut p = Parser::new(None, lx, false);
    p.get_obj()
}

fn ints(v: &[Object]) -> Vec<i32> {
    v.iter().map(|o| o.get_int()).collect()
}

#[test]
fn integers() {
    let v = lex_all(b"0 -0 +7 123 -456 2147483647 2147483648 -2147483648 -2147483649 4294967295");
    assert!(v.iter().all(|o| o.is_int()));
    assert_eq!(ints(&v), vec![0, 0, 7, 123, -456, i32::MAX, i32::MIN, i32::MIN, i32::MAX, -1]);
}

#[test]
fn adobe_number_quirks() {
    let v = lex_all(b"--123 50-100 50--100 -50-100 - + 7-");
    assert!(v.iter().all(|o| o.is_int()));
    assert_eq!(ints(&v), vec![0, 50, 50, -50, 0, 0, 7]);
    let v = lex_all(b"--123.4 50.0-100 -. . 1. .5 -.5 +.5 1.5.5");
    assert_eq!(v[0].get_real(), -(123.0 + 0.1 * 4.0));
    assert!(v[0].is_real());
    // "50.0-100": the '-' is skipped inside the fraction, so the digits
    // continue the fraction: 50.0100
    assert_eq!(v[1].get_real(), 50.0 + 0.1 * 0.0 + 0.1 * 0.1 * 1.0);
    assert!(v[2].is_real() && v[2].get_real() == 0.0 && v[2].get_real().is_sign_negative());
    assert!(v[3].is_real() && v[3].get_real() == 0.0);
    assert!(v[4].is_real() && v[4].get_real() == 1.0);
    assert_eq!(v[5].get_real(), 0.5);
    assert_eq!(v[6].get_real(), -0.5);
    assert_eq!(v[7].get_real(), 0.5);
    // "1.5.5": second '.' ends the number
    assert_eq!(v[8].get_real(), 1.5);
    assert_eq!(v[9].get_real(), 0.5);
}

#[test]
fn reals_accumulate_like_xpdf() {
    // xf = xf + scale * digit; scale *= 0.1 (not a correctly rounded
    // decimal conversion)
    let v = lex_all(b"0.1 0.7 3.14159 1.0000001");
    let acc = |int: f64, frac: &str| {
        let mut xf = int;
        let mut scale = 0.1;
        for d in frac.bytes() {
            xf += scale * (d - b'0') as f64;
            scale *= 0.1;
        }
        xf
    };
    assert_eq!(v[0].get_real().to_bits(), acc(0.0, "1").to_bits());
    assert_eq!(v[1].get_real().to_bits(), acc(0.0, "7").to_bits());
    assert_eq!(v[2].get_real().to_bits(), acc(3.0, "14159").to_bits());
    assert_eq!(v[3].get_real().to_bits(), acc(1.0, "0000001").to_bits());
}

#[test]
fn exponent_is_not_part_of_a_number() {
    let v = lex_all(b"1e5 2.5E-3");
    assert!(v[0].is_int() && v[0].get_int() == 1);
    assert!(v[1].is_cmd_of("e5"));
    assert_eq!(v[2].get_real(), 2.5);
    assert!(v[3].is_cmd_of("E-3"));
}

#[test]
fn strings_and_escapes() {
    let v = lex_all(b"(a(b)c) (\\(\\)) (\\061\\0612) (\\8) (x\\\r\ny) (x\\\ny) (x\r\ny) (x\ry) (unterminated");
    let s: Vec<Vec<u8>> = v.iter().map(|o| o.get_string().to_vec()).collect();
    assert_eq!(s[0], b"a(b)c");
    assert_eq!(s[1], b"()");
    assert_eq!(s[2], b"112");
    assert_eq!(s[3], b"8");
    assert_eq!(s[4], b"xy");
    assert_eq!(s[5], b"xy");
    assert_eq!(s[6], b"x\ny");
    assert_eq!(s[7], b"x\ny");
    assert_eq!(s[8], b"unterminated");
}

#[test]
fn hex_strings() {
    let v = lex_all(b"<48656C6c6F> <4> <> < 4 1\t4\n2 > <zz41>");
    assert_eq!(v[0].get_string(), b"Hello");
    assert_eq!(v[1].get_string(), &[0x40]);
    assert_eq!(v[2].get_string(), b"");
    assert_eq!(v[3].get_string(), b"AB");
    // illegal chars still count as digits (value 0)
    assert_eq!(v[4].get_string(), &[0x00, 0x41]);
    // unterminated
    let v = lex_all(b"<414");
    assert_eq!(v[0].get_string(), &[0x41, 0x40]);
}

#[test]
fn names() {
    let v = lex_all(b"/A /a#42c /#2F /x#4 /x#G /x#0G /Ab#00 /ok");
    assert_eq!(v[0].get_name(), b"A");
    assert_eq!(v[1].get_name(), b"aBc");
    assert_eq!(v[2].get_name(), b"/");
    assert_eq!(v[3].get_name(), b"x\x04");
    assert_eq!(v[4].get_name(), b"x#G");
    assert_eq!(v[5].get_name(), b"x");
    assert!(v[6].is_error(), "#00 makes the name an error object");
    assert_eq!(v[7].get_name(), b"ok");
    // names end at delimiters
    let v = lex_all(b"/a/b(c)/d[");
    assert_eq!(v[0].get_name(), b"a");
    assert_eq!(v[1].get_name(), b"b");
    assert_eq!(v[2].get_string(), b"c");
    assert_eq!(v[3].get_name(), b"d");
    assert!(v[4].is_cmd_of("["));
    // long names are not truncated
    let long = format!("/{}", "n".repeat(300));
    assert_eq!(lex_all(long.as_bytes())[0].get_name().len(), 300);
}

#[test]
fn commands_and_keywords() {
    let v = lex_all(b"true false null obj endobj R ) } { >x");
    assert!(matches!(v[0], Object::Bool(true)));
    assert!(matches!(v[1], Object::Bool(false)));
    assert!(v[2].is_null());
    assert!(v[3].is_cmd_of("obj"));
    assert!(v[4].is_cmd_of("endobj"));
    assert!(v[5].is_cmd_of("R"));
    assert!(v[6].is_error() && v[7].is_error() && v[8].is_error() && v[9].is_error());
    assert!(v[10].is_cmd_of("x"));
    // command tokens are limited to 127 chars; the 128th is dropped
    let long = "c".repeat(200);
    let v = lex_all(long.as_bytes());
    assert_eq!(v[0].get_cmd().len(), 127);
    assert_eq!(v[1].get_cmd().len(), 72);
}

#[test]
fn comments_and_whitespace() {
    let v = lex_all(b"%comment\r1%x\n2\x00\x0c3 % trailing");
    assert_eq!(ints(&v), vec![1, 2, 3]);
}

#[test]
fn parser_refs_and_ints() {
    let o = parse(b"[1 0 R 2 3 4 R 5 -1 R 6]");
    let a = o.get_array().unwrap();
    assert!(a.get_nf(0).is_ref());
    assert!(a.get_nf(1).is_int() && a.get_nf(1).get_int() == 2);
    assert_eq!(a.get_nf(2).get_ref(), pdfread::Ref::new(3, 4));
    // negative generation -> error object
    assert!(a.get_nf(3).is_error());
    assert_eq!(a.get_nf(4).get_int(), 6);
    assert_eq!(a.get_length(), 5);
}

#[test]
fn parser_dicts() {
    let o = parse(b"<< /A 1 /B [2 3] /A (x) 4 /C << /D /E >> /F >>");
    let d = o.get_dict().unwrap();
    // duplicate key: first position, last value; non-name key skipped
    assert_eq!(d.get_length(), 4);
    assert_eq!(&*d.get_key(0), b"A");
    assert_eq!(d.get_val(0).get_string(), b"x");
    assert_eq!(&*d.get_key(1), b"B");
    assert_eq!(&*d.get_key(2), b"C");
    assert!(d.lookup("C").dict_lookup("D").is_name_of("E"));
    // "/F >>": the value is the '>>' command and parsing continues
    assert!(d.lookup_nf("F").is_cmd_of(">>"));
}

#[test]
fn parser_unterminated() {
    let o = parse(b"[1 2 (x)");
    assert_eq!(o.array_get_length(), 3);
    let o = parse(b"<< /A 1 /B");
    assert_eq!(o.dict_get_length(), 1);
    let o = parse(b"<< /A [1 2");
    assert_eq!(o.dict_lookup("A").array_get_length(), 2);
}

#[test]
fn parser_recursion_limit() {
    let mut s = Vec::new();
    for _ in 0..600 {
        s.push(b'[');
    }
    for _ in 0..600 {
        s.push(b']');
    }
    let o = parse(&s);
    // depth 500 is reached: the innermost '[' are returned as commands
    let mut cur = o;
    let mut depth = 0;
    while let Object::Array(a) = &cur {
        let next = a.get_nf(0);
        depth += 1;
        cur = next;
    }
    assert_eq!(depth, 500);
    assert!(cur.is_cmd_of("["));
}

#[test]
fn parser_inline_image_id_quirk() {
    // an 'ID' command makes the parser skip one char and stop buffering
    let o = parse(b"[1 ID xyz 2]");
    let a = o.get_array().unwrap();
    assert_eq!(a.get_nf(0).get_int(), 1);
    assert!(a.get_nf(1).is_cmd_of("ID"));
}
