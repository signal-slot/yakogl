// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

//! The V3D 7.1 no_std scan (docs/notes/v3d71-nostd.md section 4, item N1, rule
//! P1). Every file of `FILES` must be core-only: none of the P1 tokens
//! (`Vec`, `String`, `Box`, `vec!`, `format!`, `to_string`, `to_vec`,
//! `collect`, `std::`) may appear outside an item gated by
//! `#[cfg(feature = "host-test")]`. Comments and string literals are skipped
//! by a small hand-written lexer (no dependencies). Later items add their
//! files to `FILES`.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_nostd_scan`

/// The files under src/v3d/ the scan covers. Append here.
const FILES: &[&str] = &[
    "admit71.rs",
    "record71.rs",
    "attr71.rs",
    "clearplan71.rs",
    "ffi71.rs",
    "format71.rs",
    "ident71.rs",
    "limits71.rs",
    "rt71.rs",
    "uniform71.rs",
    "zskeep71.rs",
    "texuni71.rs",
    "sampler71.rs",
    "texstate71.rs",
];

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Punct(char),
}

/// Tokens with their 1-based line; comments dropped, string and char
/// literals reduced to `Str`.
fn lex(src: &str) -> Vec<(Tok, usize)> {
    let b: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line) = (0usize, 1usize);
    let at = |i: usize| b.get(i).copied().unwrap_or('\0');
    while i < b.len() {
        let c = b[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && at(i + 1) == '/' {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == '*' {
            let mut depth = 1;
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == '/' && at(i + 1) == '*' {
                    depth += 1;
                    i += 2;
                } else if b[i] == '*' && at(i + 1) == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    if b[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if c == '"' || ((c == 'b') && at(i + 1) == '"') {
            if c == 'b' {
                i += 1;
            }
            i += 1;
            let start = i;
            let l0 = line;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' {
                    i += 1;
                }
                if at(i) == '\n' {
                    line += 1;
                }
                i += 1;
            }
            out.push((Tok::Str(b[start..i.min(b.len())].iter().collect()), l0));
            i += 1;
        } else if (c == 'r' || (c == 'b' && at(i + 1) == 'r')) && {
            let mut j = i + if c == 'b' { 2 } else { 1 };
            while at(j) == '#' {
                j += 1;
            }
            at(j) == '"'
        } {
            let mut j = i + if c == 'b' { 2 } else { 1 };
            let mut hashes = 0;
            while at(j) == '#' {
                hashes += 1;
                j += 1;
            }
            j += 1;
            let start = j;
            let l0 = line;
            loop {
                if j >= b.len() {
                    break;
                }
                if b[j] == '"' && (0..hashes).all(|k| at(j + 1 + k) == '#') {
                    break;
                }
                if b[j] == '\n' {
                    line += 1;
                }
                j += 1;
            }
            out.push((Tok::Str(b[start..j.min(b.len())].iter().collect()), l0));
            i = j + 1 + hashes;
        } else if c == '\'' {
            // A char literal ('x', '\n', '\'') or a lifetime ('a).
            if at(i + 1) == '\\' {
                i += 2;
                while i < b.len() && b[i] != '\'' {
                    i += 1;
                }
                i += 1;
                out.push((Tok::Str(String::new()), line));
            } else if at(i + 2) == '\'' {
                i += 3;
                out.push((Tok::Str(String::new()), line));
            } else {
                i += 1; // a lifetime: its name lexes as an identifier
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while at(i).is_alphanumeric() || at(i) == '_' {
                i += 1;
            }
            out.push((Tok::Ident(b[start..i].iter().collect()), line));
        } else if c.is_ascii_digit() {
            // A number, with its suffix and any `1e5`/`0x..`: one token.
            let start = i;
            while at(i).is_alphanumeric() || at(i) == '_' {
                i += 1;
            }
            out.push((Tok::Ident(b[start..i].iter().collect()), line));
        } else {
            out.push((Tok::Punct(c), line));
            i += 1;
        }
    }
    out
}

fn is_punct(t: &(Tok, usize), c: char) -> bool {
    t.0 == Tok::Punct(c)
}

/// Does the attribute whose `[` is at `open` gate its item on host-test?
/// Returns (gated, index after the closing `]`).
fn attr_gate(t: &[(Tok, usize)], open: usize) -> (bool, usize) {
    let mut depth = 0;
    let mut i = open;
    while i < t.len() {
        if is_punct(&t[i], '[') {
            depth += 1;
        } else if is_punct(&t[i], ']') {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        i += 1;
    }
    let body = &t[open..i.min(t.len())];
    let has = |name: &str| body.iter().any(|x| x.0 == Tok::Ident(name.into()));
    let host = body.iter().any(|x| x.0 == Tok::Str("host-test".into()));
    (has("cfg") && host && !has("not") && !has("any"), i + 1)
}

/// The end (exclusive) of the item starting at `i`: past the first
/// top-level `{ .. }` block, or past the first top-level `;`.
fn item_end(t: &[(Tok, usize)], mut i: usize) -> usize {
    let mut paren = 0i32;
    while i < t.len() {
        match &t[i].0 {
            Tok::Punct('(') | Tok::Punct('[') => paren += 1,
            Tok::Punct(')') | Tok::Punct(']') => paren -= 1,
            Tok::Punct(';') if paren == 0 => return i + 1,
            Tok::Punct('{') if paren == 0 => {
                let mut d = 0;
                while i < t.len() {
                    if is_punct(&t[i], '{') {
                        d += 1;
                    } else if is_punct(&t[i], '}') {
                        d -= 1;
                        if d == 0 {
                            return i + 1;
                        }
                    }
                    i += 1;
                }
                return t.len();
            }
            _ => {}
        }
        i += 1;
    }
    t.len()
}

/// Every P1 token outside a host-test item, as "line: token".
fn scan(src: &str) -> Vec<String> {
    let t = lex(src);
    let mut hits = Vec::new();
    let mut i = 0;
    while i < t.len() {
        // An inner `#![cfg(feature = "host-test")]` gates the whole file.
        if is_punct(&t[i], '#') && i + 2 < t.len() && is_punct(&t[i + 1], '!') && is_punct(&t[i + 2], '[') {
            let (gated, next) = attr_gate(&t, i + 2);
            if gated {
                return hits;
            }
            i = next;
            continue;
        }
        if is_punct(&t[i], '#') && i + 1 < t.len() && is_punct(&t[i + 1], '[') {
            // One or more outer attributes, then the item they gate.
            let mut gated = false;
            let mut j = i;
            while j + 1 < t.len() && is_punct(&t[j], '#') && is_punct(&t[j + 1], '[') {
                let (g, next) = attr_gate(&t, j + 1);
                gated |= g;
                j = next;
            }
            if gated {
                i = item_end(&t, j);
            } else {
                i = j;
            }
            continue;
        }
        if let Tok::Ident(name) = &t[i].0 {
            let next_is = |c: char| t.get(i + 1).is_some_and(|x| is_punct(x, c));
            let bad = match name.as_str() {
                "Vec" | "String" | "Box" | "to_string" | "to_vec" | "collect" => true,
                "vec" | "format" => next_is('!'),
                "std" => next_is(':'),
                _ => false,
            };
            if bad {
                hits.push(format!("{}: {}", t[i].1, name));
            }
        }
        i += 1;
    }
    hits
}

fn read(name: &str) -> String {
    let p = format!("{}/src/v3d/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

#[test]
fn listed_leaves_are_core_only() {
    let mut bad = Vec::new();
    for f in FILES {
        for h in scan(&read(f)) {
            bad.push(format!("src/v3d/{f}:{h}"));
        }
    }
    assert!(bad.is_empty(), "alloc/std tokens outside host-test items:\n{}", bad.join("\n"));
}

#[test]
fn list_has_no_duplicates_and_files_exist() {
    for (n, f) in FILES.iter().enumerate() {
        assert!(!FILES[..n].contains(f), "{f} listed twice");
        assert!(!read(f).is_empty(), "{f} is empty");
    }
}

#[test]
fn scan_flags_every_p1_token() {
    for (src, tok) in [
        ("fn f() { let v: Vec<u8> = x; }", "Vec"),
        ("fn f() -> String { x }", "String"),
        ("fn f() { let b = Box::new(1); }", "Box"),
        ("fn f() { let v = vec![1]; }", "vec"),
        ("fn f() { let s = format!(\"x\"); }", "format"),
        ("fn f() { x.to_string(); }", "to_string"),
        ("fn f() { x.to_vec(); }", "to_vec"),
        ("fn f() { x.iter().collect::<u8>(); }", "collect"),
        ("use std::fmt;", "std"),
    ] {
        let h = scan(src);
        assert_eq!(h.len(), 1, "{src}: {h:?}");
        assert!(h[0].ends_with(tok), "{src}: {h:?}");
    }
}

#[test]
fn scan_skips_comments_strings_and_host_test_items() {
    let src = r##"
// Vec in a line comment
/* String /* Box nested */ vec! */
const A: &str = "Vec String std::fmt format!";
const B: &str = r#"Box "to_string" "#;
const C: char = '"';
fn lt<'a>(x: &'a u8) -> &'a u8 { x } // 'a is a lifetime
#[cfg(feature = "host-test")]
fn gated() -> Vec<u8> { vec![1; 3] }
#[cfg(all(feature = "host-test", test))]
mod m { use std::fmt; fn g() -> String { String::new() } }
#[cfg(feature = "host-test")]
#[derive(Debug)]
struct S(Vec<u8>);
#[cfg(feature = "host-test")]
const T: [u8; 2] = [0; 2];
fn clean() -> u8 { 0 }
"##;
    assert_eq!(scan(src), Vec::<String>::new());
}

#[test]
fn scan_does_not_skip_other_cfgs_or_negated_host_test() {
    assert_eq!(scan("#[cfg(test)] fn f() -> Vec<u8> { x }").len(), 1);
    assert_eq!(scan("#[cfg(not(feature = \"host-test\"))] fn f() -> Vec<u8> { x }").len(), 1);
    // The gated item ends with its block: what follows is scanned again.
    assert_eq!(scan("#[cfg(feature = \"host-test\")] fn f() { } fn g() -> Box<u8> { x }").len(), 1);
}
