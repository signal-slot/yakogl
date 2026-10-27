// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES 1.00 preprocessor against dEQP-GLES2 `preprocessor.test`: the
//! expanded token stream of a case is compared with the expected one.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es2_preprocessor
//! ```
#![cfg(feature = "shaderc-online")]
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::lexer::Tok;
use yakogl::shaderc::online::pp::preprocess;

fn expand(src: &str) -> Vec<String> {
    let mut buf = vec![0u8; 1 << 20];
    let a = Arena::new(&mut buf);
    match preprocess(&a, src) {
        Ok(r) => r
            .tokens
            .iter()
            .filter(|t| t.kind != Tok::Eof)
            .map(|t| {
                // A predefined macro keeps its name as text; its value is in `int`.
                if t.kind == Tok::IntLit && t.text.starts_with('_') {
                    t.int.to_string()
                } else {
                    t.text.to_string()
                }
            })
            .collect(),
        Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
    }
}

#[test]
fn define_value_and_function() {
    let src = "precision mediump float;\n#    define        VALUE            (1.5 + 2.5)\n#    define        FUNCTION(__LINE__, b)    __LINE__+b\nvoid main()\n{\n    out0 = FUNCTION(VALUE, ((0.2) + 1.8) );\n}\n";
    let t = expand(src);
    let s = t.join(" ");
    assert!(s.contains("out0 = ( 1.5 + 2.5 ) + ( ( 0.2 ) + 1.8 ) ;"), "{s}");
}

/// dEQP gives the same body to both stages (`both`); a parameter named like a
/// predefined macro is a parameter inside the body, and `__LINE__` outside it
/// is still the line.
#[test]
fn define_value_and_function_vertex_and_fragment() {
    let body = "#define VALUE (1.5 + 2.5)\n#define FUNCTION(__LINE__, b) __LINE__+b\nout0 = FUNCTION(VALUE, ((0.2) + 1.8) );\n";
    let want = "out0 = ( 1.5 + 2.5 ) + ( ( 0.2 ) + 1.8 ) ;";
    assert_eq!(expand(body).join(" "), want);
    assert_eq!(expand(&format!("{body}x = __LINE__;")).join(" "), format!("{want} x = 4 ;"));
}

#[test]
fn expansion_order() {
    // An object-like macro naming a function-like one, called after rescanning.
    assert_eq!(expand("#define F(x) x+1\n#define G F\nG(2);").join(" "), "2 + 1 ;");
    // An argument is expanded before substitution; the result is rescanned.
    assert_eq!(expand("#define F(x) x*x\n#define A 3\nF(A);").join(" "), "3 * 3 ;");
    assert_eq!(expand("#define F(x) x\n#define G(x) F(x)+1\nG(F(2));").join(" "), "2 + 1 ;");
}

/// The tail rescan is a loop, not a recursion: a long body of uses stays flat.
#[test]
fn tail_rescan_does_not_nest() {
    let mut src = String::from("#define F(x) x+1\n#define G F\n");
    for _ in 0..100 {
        src.push_str("G(2);\n");
    }
    assert_eq!(expand(&src).join(" "), vec!["2 + 1 ;"; 100].join(" "));
}

/// Only a name the expansion itself produced is invoked: an empty expansion
/// leaves the name before it alone (C99 6.10.3.4), even across a directive.
#[test]
fn empty_expansion_does_not_invoke_the_name_before_it() {
    assert_eq!(expand("#define F(x) [x]\n#define E\nF E (2);").join(" "), "F ( 2 ) ;");
    assert_eq!(expand("#define F(x) [x]\nF\n#define E\nE (2);").join(" "), "F ( 2 ) ;");
}

/// C99's `f(2)(9)` example. ES 1.00 makes recursion an error and `g(9)`
/// reaches `g` again through `f`, so it is refused rather than `2*9*g`.
#[test]
fn mutual_recursion_through_a_tail_name_is_an_error() {
    let mut buf = vec![0u8; 1 << 20];
    let a = Arena::new(&mut buf);
    assert!(preprocess(&a, "#define f(a) a*g\n#define g(a) f(a)\nf(2)(9);\n").is_err());
}
