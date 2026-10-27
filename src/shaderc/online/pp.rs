// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GLSL ES 1.00 preprocessor (specification section 3.4).
//!
//! It takes the token stream [`super::lexer`] produced and returns another
//! one with the directives removed, the conditional groups resolved and the
//! macros expanded -- what the parser sees.
//!
//! # Where the rules come from
//!
//! The specification's prose leaves several questions open that an
//! implementation must answer the same way glslang does, or shaders that must
//! compile will not and shaders that must fail will pass. dEQP-GLES2 settles
//! them: `data/gles2/shaders/preprocessor.test` is 267 cases, 130 of them
//! `expect compile_fail`, and it is the reference this module was written
//! against. Four answers it pins, each of which a plain reading of C would
//! get wrong:
//!
//! * **Redefinition must be identical down to where the white space is.**
//!   `object_redefinitions.invalid_object_whitespace` defines `VALUE` as
//!   `(AAA - 1.0)` and then `(AAA- 1.0)` and expects a compile failure, while
//!   `redefine_object_comment` redefines it with a *different comment* in the
//!   same place and expects success. That is C99 6.10.3p2 exactly: the
//!   replacement lists must have the same tokens with white space in the same
//!   gaps, and all white space counts as the same white space. The lexer's
//!   `ws_before` flag is what makes the comparison a one-liner.
//! * **A recursive macro is an error**, not a fixed point. `#define AAA AAA`
//!   is `recursion.recursion_1`, `expect compile_fail`. C's "blue paint" rule
//!   would quietly accept it.
//! * **An undefined identifier in a live `#if` is an error**, but a
//!   short-circuited one is not: `#if A && B` fails
//!   (`undefined_identifiers.undefined_identifier_5`) while `#if 1 || AAA`
//!   passes (`valid_undefined_identifier_1`). C would substitute 0 for both.
//! * **A name with a double underscore is allowed.** The specification
//!   reserves it, but `basic.identifier_with_double_underscore` defines
//!   `__VALUE__` and expects success, so rejecting it would fail a must-pass
//!   case. Undefining a *predefined* macro is the error
//!   (`definitions.undefine_invalid_object_1`, `#undef __LINE__`).

use super::arena::{AVec, Arena};
use super::lexer::{tokenize, Tok, Token, Version};
use super::{Error, R};

/// How deep macro expansion may nest before the compiler gives up.
///
/// Recursion is already an error, so this only catches a chain of distinct
/// macros. It exists because the alternative to a limit is an unbounded
/// recursion on a target with a 16 KiB stack.
const MAX_EXPANSION_DEPTH: u32 = 64;

/// The predefined macros (section 3.4). They are not in the macro table:
/// `#define` and `#undef` of any of them is an error, and `__LINE__` has a
/// different value at every use.
const PREDEFINED: [&str; 4] = ["__LINE__", "__FILE__", "__VERSION__", "GL_ES"];

/// Section 4.5.4: "The built-in macro GL_FRAGMENT_PRECISION_HIGH is defined
/// to one on systems supporting highp precision in the fragment language
/// [...] When defined, this macro is available in both the vertex and fragment
/// languages." Whether this system does is [`crate::backend::FRAGMENT_HIGHP`],
/// the fact glGetShaderPrecisionFormat reports.
const FRAGMENT_PRECISION_HIGH: &str = "GL_FRAGMENT_PRECISION_HIGH";

/// A predefined macro: section 3.4's, and section 4.5.4's when this system
/// has fragment highp. Like GL_ES it cannot be defined or undefined.
fn is_predefined(name: &str) -> bool {
    PREDEFINED.contains(&name) || (crate::backend::FRAGMENT_HIGHP && name == FRAGMENT_PRECISION_HIGH)
}

/// What [`preprocess`] produced.
pub struct Preprocessed<'a> {
    /// The parser's input: directives gone, macros expanded, one `Eof`.
    pub tokens: &'a [Token<'a>],
    /// The version the `#version` directive asked for, or 100.
    pub version: u32,
    /// The lexical rules that were applied.
    pub lexer_version: Version,
    /// `#pragma STDGL invariant(all)` was seen in live code: every output is
    /// invariant (section 4.6.1).
    pub all_invariant: bool,
}

/// One `#define`.
#[derive(Clone, Copy)]
struct Macro<'a> {
    name: &'a str,
    /// Parameter names; empty for an object-like macro.
    params: &'a [&'a str],
    /// `#define A(x)` versus `#define A (x)`: the difference is whether the
    /// `(` touched the name, and it changes what the macro is.
    function_like: bool,
    body: &'a [Token<'a>],
}

/// One level of `#if` nesting.
#[derive(Clone, Copy)]
struct Cond {
    /// This group's tokens are being kept.
    active: bool,
    /// Some branch of this `#if` chain has already been taken, so no later
    /// `#elif` or `#else` may be.
    taken: bool,
    /// `#else` has been seen: another `#else` or `#elif` is an error.
    saw_else: bool,
    /// The enclosing group was active. A nested `#if` inside a dead group
    /// stays dead whatever its condition says -- and its condition is not
    /// evaluated, so an undefined identifier in it is not an error.
    outer_active: bool,
    /// Where the `#if` was, for the "unterminated" diagnostic.
    line: u32,
}

/// Run the preprocessor over `src`.
///
/// Everything is built in `a`; the returned tokens borrow it and the source.
pub fn preprocess<'a>(a: &Arena<'a>, src: &'a str) -> R<'a, Preprocessed<'a>> {
    let (version, lexer_version) = scan_version(src)?;
    if let Some(why) = out_of_scope(version) {
        /* A dialect this build does not implement is refused here, at the
         * line that says so, which is the difference between a fact and a
         * false report: parsed with the wrong rules such a shader stops on
         * some later construct with a message that reads as a missing
         * feature rather than as a shader the build is not asked to accept.
         *
         * GLSL ES 3.00 is in scope since 2026-09-23 (Qt Quick 3D takes the
         * ES 3.0 path on both boards), except in a `report-es2` build, which
         * tells the application it is ES 2.0 and so compiles what ES 2.0
         * compiles. 3.10 and 3.20 are out of scope in every build. */
        return Err(Error::new(1, "#version", why));
    }
    let toks = tokenize(a, src, lexer_version)?;
    let mut pp = Pp {
        a,
        toks,
        i: 0,
        version,
        macros: AVec::new(),
        cond: AVec::new(),
        out: AVec::with_capacity(a, toks.len())?,
        pending: AVec::new(),
        active_macros: AVec::new(),
        seen_non_directive: false,
        seen_version: false,
        line_delta: 0,
        file: 0,
        invocation_line: None,
        all_invariant: false,
    };
    pp.run()?;
    let tokens = pp.out.into_slice();
    /* The lexer's tokens are consumed: the output copies every token it
     * keeps (a token's text is the source's, or the arena's for a spliced
     * or computed one, never the lexer buffer's), and the macros that held
     * slices of it end with `pp`. Given back, so the phases after this one
     * reuse the space instead of stacking on top of it. */
    // SAFETY: as above; nothing refers to `toks` past this point.
    unsafe { a.give_back(toks) };
    let all_invariant = pp.all_invariant;
    Ok(Preprocessed { tokens, version, lexer_version, all_invariant })
}

/// Find the `#version` directive before tokenizing, because it decides which
/// lexical rules apply (ES 3.00 has line continuation and literal suffixes,
/// ES 1.00 has neither) and the lexer cannot be told after the fact.
///
/// Only the directive's own shape is read here. Whether it is allowed to be
/// where it is -- first in the shader, but for comments and white space -- is
/// checked during the real pass, where the tokens around it are available for
/// the diagnostic.
fn scan_version(src: &str) -> R<'_, (u32, Version)> {
    let b = src.as_bytes();
    let mut i = 0usize;
    /* Skip white space and comments by hand: the lexer cannot run yet, since
     * what it does with a backslash is the very thing being decided. */
    loop {
        match b.get(i) {
            Some(&c) if c.is_ascii_whitespace() => i += 1,
            Some(b'/') if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            Some(b'/') if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
            }
            _ => break,
        }
    }
    if b.get(i) != Some(&b'#') {
        /* No `#version` at all is GLSL ES 1.00 (section 3.3), not a guess: a
         * vertex shader that writes gl_Position from constants contains no
         * `attribute`, `varying` or `gl_FragColor` to sniff for, and dEQP's
         * pragma cases are exactly that shape. */
        return Ok((100, Version::Es100));
    }
    i += 1;
    while b.get(i).is_some_and(|c| *c == b' ' || *c == b'\t') {
        i += 1;
    }
    if !src[i..].starts_with("version") {
        return Ok((100, Version::Es100));
    }
    i += "version".len();
    while b.get(i).is_some_and(|c| *c == b' ' || *c == b'\t') {
        i += 1;
    }
    let start = i;
    while b.get(i).is_some_and(|c| c.is_ascii_digit()) {
        i += 1;
    }
    if start == i {
        return Err(Error::new(1, "#version", "#version requires a version number"));
    }
    let mut n: u32 = 0;
    for &d in &b[start..i] {
        n = n.saturating_mul(10).saturating_add((d - b'0') as u32);
    }
    while b.get(i).is_some_and(|c| *c == b' ' || *c == b'\t') {
        i += 1;
    }
    let es = src[i..].starts_with("es");
    match (n, es) {
        (100, false) => Ok((100, Version::Es100)),
        (300, true) | (310, true) | (320, true) => Ok((n, Version::Es300)),
        /* Anything else. A version WITHOUT `es` that is not 100 is desktop
         * GLSL, and saying so matters here: the offline tables are built
         * from sources written in GLSL 450 with explicit locations (the
         * generators accept both spellings), so every one of those is a
         * correct refusal by an ES 1.00 compiler and NOT a gap. Eighteen
         * sources under tools/shaders are that shape, and the twin-draw
         * survey carried them for a while as an unattributed bucket called
         * "unsupported #version". */
        /* An ES version number with the `es` left off is a typo, not
         * desktop GLSL, and there is no desktop GLSL 300/310/320 to
         * confuse it with -- desktop runs 110, 120, 130, 140, 150, 330 and
         * up. Saying "desktop" there would send the author looking in the
         * wrong direction. */
        (300 | 310 | 320, false) => Err(Error::new(1, "#version", "an ES version number needs 'es' after it")),
        (_, false) => Err(Error::new(1, "#version", "desktop GLSL source; this is an OpenGL ES compiler")),
        _ => Err(Error::new(1, "#version", "unsupported shading language version")),
    }
}

/// Why a shader of `version` is refused, naming the dialect it is written in,
/// or `None` when this build compiles it. The dialect is part of the fact:
/// "GLSL ES 3.10 source" tells a caller what to do about it, "unsupported
/// version" does not.
fn out_of_scope(version: u32) -> Option<&'static str> {
    match version {
        310 => Some("GLSL ES 3.10 source; this compiler implements GLSL ES 1.00 and 3.00"),
        320 => Some("GLSL ES 3.20 source; this compiler implements GLSL ES 1.00 and 3.00"),
        300 if cfg!(feature = "report-es2") => Some("GLSL ES 3.00 source; this build reports OpenGL ES 2.0"),
        _ => None,
    }
}

struct Pp<'x, 'a> {
    a: &'x Arena<'a>,
    toks: &'a [Token<'a>],
    i: usize,
    version: u32,
    macros: AVec<'a, Macro<'a>>,
    cond: AVec<'a, Cond>,
    out: AVec<'a, Token<'a>>,
    /// Source tokens waiting to be macro-expanded. They are held until a
    /// directive or the end of the file, so that a function-like macro
    /// invocation whose arguments span several lines still works.
    pending: AVec<'a, Token<'a>>,
    /// Macros currently being expanded, innermost last. A name that appears
    /// here again is a recursive macro, which ES 1.00 makes an error.
    active_macros: AVec<'a, &'a str>,
    seen_non_directive: bool,
    seen_version: bool,
    /// `#pragma STDGL invariant(all)`; see [`Preprocessed::all_invariant`].
    all_invariant: bool,
    /// What `#line` did: added to a token's own line to give `__LINE__`
    /// (section 3.4, "the following line has line number line").
    /// Diagnostics keep the application's own lines, which is what an info
    /// log's `0:<line>:` means and what dEQP reads.
    line_delta: i64,
    /// `#line`'s source-string number, which `__FILE__` substitutes.
    file: u32,
    /// The line a macro invocation being expanded ENDS on, while its
    /// replacement list is expanded: `__LINE__` there is that line, not the
    /// line of the `#define` the token was written on. The outermost
    /// invocation decides, because nothing is substituted until it has been
    /// read to its `)`. Arguments are replaced first (section 3.4), before
    /// this is set, and keep their own lines -- dEQP's
    /// predefined_macros.line_2 expects exactly that mix.
    invocation_line: Option<u32>,
}

impl<'a> Pp<'_, 'a> {
    fn run(&mut self) -> R<'a, ()> {
        loop {
            let t = self.toks[self.i];
            if t.kind == Tok::Eof {
                self.flush_pending()?;
                if let Some(c) = self.cond.as_slice().last() {
                    return Err(Error::new(c.line, "#if", "#if without #endif"));
                }
                self.out.push(self.a, t)?;
                return Ok(());
            }
            if t.bol && t.kind == Tok::Hash {
                self.flush_pending()?;
                self.directive()?;
                continue;
            }
            if self.active() {
                if t.kind == Tok::Hash || t.kind == Tok::HashHash {
                    return Err(Error::new(t.line, t.text, "'#' is only allowed at the start of a line"));
                }
                if t.kind == Tok::Unknown {
                    /* The lexer does not refuse a character outside the
                     * source set, because `#pragma` may hold one; live code
                     * may not, and this is where that line is drawn. */
                    return Err(Error::new(t.line, t.text, "illegal character"));
                }
                self.seen_non_directive = true;
                self.pending.push(self.a, t)?;
            }
            self.i += 1;
        }
    }

    fn active(&self) -> bool {
        self.cond.as_slice().last().is_none_or(|c| c.active)
    }

    /// Expand and emit everything buffered since the last directive.
    fn flush_pending(&mut self) -> R<'a, ()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let toks = core::mem::take(&mut self.pending).into_slice();
        let mut out = core::mem::take(&mut self.out);
        let r = self.expand_into(toks, &mut out, 0);
        self.out = out;
        r
    }

    /* ---- directives ---------------------------------------------------- */

    /// The tokens of the logical line starting at `self.i`, and the index
    /// after it. A directive ends at the next token that begins a line.
    fn line_tokens(&self) -> (&'a [Token<'a>], usize) {
        let start = self.i;
        let mut j = start + 1;
        while self.toks[j].kind != Tok::Eof && !self.toks[j].bol {
            j += 1;
        }
        (&self.toks[start..j], j)
    }

    fn directive(&mut self) -> R<'a, ()> {
        let (line, next) = self.line_tokens();
        self.i = next;
        let rest = &line[1..];
        let Some(name) = rest.first() else {
            /* `#` alone is the null directive; section 3.4 allows it. */
            return Ok(());
        };
        if name.kind != Tok::Ident {
            if self.active() {
                return Err(Error::new(name.line, name.text, "invalid preprocessor directive"));
            }
            return Ok(());
        }
        let body = &rest[1..];
        match name.text {
            /* The conditional directives run whether or not the group is
             * active: a dead group's nesting still has to be tracked. */
            "if" => self.d_if(name, body),
            "ifdef" => self.d_ifdef(name, body, false),
            "ifndef" => self.d_ifdef(name, body, true),
            "elif" => self.d_elif(name, body),
            "else" => self.d_else(name, body),
            "endif" => self.d_endif(name, body),
            _ if !self.active() => Ok(()),
            "define" => self.d_define(name, body),
            "undef" => self.d_undef(name, body),
            "version" => self.d_version(name, body),
            "error" => Err(Error::new(name.line, body.first().map_or("", |t| t.text), "#error")),
            "pragma" => {
                /* Section 4.6.1's `#pragma STDGL invariant(all)`; any other
                 * pragma is unrecognised and ignored (section 3.4). */
                let words = ["STDGL", "invariant", "(", "all", ")"];
                if body.len() == words.len() && body.iter().zip(words).all(|(t, w)| t.text == w) {
                    self.all_invariant = true;
                }
                Ok(())
            }
            "extension" => self.d_extension(name, body),
            "line" => self.d_line(name, body),
            _ => Err(Error::new(name.line, name.text, "invalid preprocessor directive")),
        }
    }

    fn end_of_directive(&self, body: &'a [Token<'a>], n: usize) -> R<'a, ()> {
        match body.get(n) {
            None => Ok(()),
            Some(t) => Err(Error::new(t.line, t.text, "extra tokens at the end of the directive")),
        }
    }

    fn d_version(&mut self, name: &Token<'a>, body: &'a [Token<'a>]) -> R<'a, ()> {
        if self.seen_version || self.seen_non_directive || !self.cond.is_empty() || !self.macros.is_empty() {
            return Err(Error::new(name.line, "#version", "#version must be the first directive in the shader"));
        }
        self.seen_version = true;
        let Some(num) = body.first() else {
            return Err(Error::new(name.line, "#version", "#version requires a version number"));
        };
        if num.kind != Tok::IntLit {
            return Err(Error::new(num.line, num.text, "#version requires a version number"));
        }
        let n = if body.len() > 1 { 2 } else { 1 };
        if body.len() > 1 && !(body[1].kind == Tok::Ident && body[1].text == "es") {
            return Err(Error::new(body[1].line, body[1].text, "expected 'es' after the version number"));
        }
        self.end_of_directive(body, n)
    }

    fn d_define(&mut self, name: &Token<'a>, body: &'a [Token<'a>]) -> R<'a, ()> {
        let Some(id) = body.first() else {
            return Err(Error::new(name.line, "#define", "#define requires a macro name"));
        };
        if id.kind != Tok::Ident {
            return Err(Error::new(id.line, id.text, "#define requires a macro name"));
        }
        /* `defined` is the `#if` operator, not a macro. ES 1.00 is refused
         * here as before (C calls it undefined, and no ES 1.00 case needs
         * it); in ES 3.00 dEQP's conditional_inclusion.define_defined and
         * define_defined_outside_if define it and must BUILD. In an `#if`
         * the operator still wins (`fold_defined` looks for the name before
         * anything is expanded); elsewhere it expands like any macro. */
        if is_predefined(id.text) || (id.text == "defined" && !id.es3) {
            return Err(Error::new(id.line, id.text, "cannot redefine a predefined macro"));
        }
        /* Section 3.4: "All macro names prefixed with 'GL_' ('GL' followed by
         * a single underscore) are also reserved."
         *
         * Only this half of the sentence is enforced. The other half reserves
         * names containing two consecutive underscores, and dEQP's
         * `basic.identifier_with_double_underscore` defines `__VALUE__` and
         * expects SUCCESS, so refusing it would fail a must-pass case -- see
         * the module note at the top of this file. dEQP is the oracle we
         * have, and where it contradicts the specification the must-pass
         * side wins, because that is the side that breaks working shaders. */
        if id.text.starts_with("GL_") {
            return Err(Error::new(id.line, id.text, "'GL_' is a reserved macro prefix"));
        }
        let mut rest = &body[1..];
        let mut params: AVec<&'a str> = AVec::new();
        let function_like = rest.first().is_some_and(|t| t.kind == Tok::LParen && !t.ws_before);
        if function_like {
            rest = &rest[1..];
            /* `A()` is a macro of no parameters; anything else is a
             * comma-separated list of distinct identifiers. */
            if rest.first().is_some_and(|t| t.kind == Tok::RParen) {
                rest = &rest[1..];
            } else {
                loop {
                    let Some(p) = rest.first() else {
                        return Err(Error::new(id.line, id.text, "missing ')' in the macro parameter list"));
                    };
                    if p.kind != Tok::Ident {
                        return Err(Error::new(p.line, p.text, "macro parameter must be an identifier"));
                    }
                    if params.as_slice().contains(&p.text) {
                        return Err(Error::new(p.line, p.text, "duplicate macro parameter"));
                    }
                    params.push(self.a, p.text)?;
                    rest = &rest[1..];
                    match rest.first().map(|t| t.kind) {
                        Some(Tok::Comma) => rest = &rest[1..],
                        Some(Tok::RParen) => {
                            rest = &rest[1..];
                            break;
                        }
                        _ => return Err(Error::new(id.line, id.text, "missing ')' in the macro parameter list")),
                    }
                }
            }
        }
        let mac = Macro { name: id.text, params: params.into_slice(), function_like, body: rest };
        if let Some(old) = self.find_macro(id.text) {
            /* C99 6.10.3p2, which dEQP's object_redefinitions group checks:
             * a redefinition is benign only when the replacement list is
             * identical, white space included. */
            if !same_definition(&old, &mac) {
                return Err(Error::new(id.line, id.text, "macro redefined with a different definition"));
            }
            return Ok(());
        }
        self.macros.push(self.a, mac)?;
        Ok(())
    }

    fn d_undef(&mut self, name: &Token<'a>, body: &'a [Token<'a>]) -> R<'a, ()> {
        let Some(id) = body.first() else {
            return Err(Error::new(name.line, "#undef", "#undef requires a macro name"));
        };
        if id.kind != Tok::Ident {
            return Err(Error::new(id.line, id.text, "#undef requires a macro name"));
        }
        if is_predefined(id.text) {
            return Err(Error::new(id.line, id.text, "cannot undefine a predefined macro"));
        }
        self.end_of_directive(body, 1)?;
        /* Undefining a macro that is not defined is allowed and does nothing
         * (section 3.4). */
        let mut keep = AVec::with_capacity(self.a, self.macros.len())?;
        for m in self.macros.as_slice() {
            if m.name != id.text {
                keep.push(self.a, *m)?;
            }
        }
        self.macros = keep;
        Ok(())
    }

    fn d_extension(&mut self, name: &Token<'a>, body: &'a [Token<'a>]) -> R<'a, ()> {
        /* Section 3.4: "If nothing is said, the granularity is a single
         * compilation unit, and the extension directives must occur before
         * any non-preprocessor tokens."
         *
         * `seen_non_directive` already exists for `#version`, which has the
         * stricter form of the same rule, and it means exactly what this
         * one needs: a token that is not part of a directive has been read.
         * dEQP `extensions.after_non_preprocessing_tokens` puts the second
         * `#extension` INSIDE `main`, which is the shape that matters --
         * there is no scope smaller than the compilation unit for it to
         * apply to. */
        if self.seen_non_directive {
            return Err(Error::new(name.line, "#extension", "#extension must come before any other token"));
        }
        let Some(ext) = body.first() else {
            return Err(Error::new(name.line, "#extension", "#extension requires an extension name"));
        };
        if ext.kind != Tok::Ident {
            return Err(Error::new(ext.line, ext.text, "#extension requires an extension name"));
        }
        match body.get(1) {
            Some(t) if t.kind == Tok::Colon => {}
            Some(t) => return Err(Error::new(t.line, t.text, "expected ':' after the extension name")),
            None => return Err(Error::new(ext.line, ext.text, "expected ':' after the extension name")),
        }
        let Some(beh) = body.get(2) else {
            return Err(Error::new(ext.line, ext.text, "#extension requires a behavior"));
        };
        if beh.kind != Tok::Ident || !matches!(beh.text, "require" | "enable" | "warn" | "disable") {
            return Err(Error::new(beh.line, beh.text, "invalid #extension behavior"));
        }
        self.end_of_directive(body, 3)?;
        /* `require` on an extension this implementation does not advertise
         * must fail (section 3.4); `enable` and `warn` only warn, and
         * `disable` is always fine. `all` may not be required or enabled.
         *
         * What counts as advertised is `src/gl/state.rs`'s own list -- the
         * one `glGetString(GL_EXTENSIONS)` returns -- rather than a copy
         * here. A second list would drift: the compiler would refuse a
         * shader the implementation claims to support, or accept one it does
         * not, and neither would show up until an application hit it. */
        if ext.text == "all" {
            if matches!(beh.text, "require" | "enable") {
                return Err(Error::new(beh.line, beh.text, "'all' cannot be required or enabled"));
            }
            return Ok(());
        }
        if beh.text == "require" && !crate::gl::state::advertises(ext.text) {
            return Err(Error::new(ext.line, ext.text, "extension is not supported"));
        }
        Ok(())
    }

    fn d_line(&mut self, name: &Token<'a>, body: &'a [Token<'a>]) -> R<'a, ()> {
        /* The expression is macro-expanded first (section 3.4), and "after
         * processing this directive (including its new-line), the
         * implementation will behave as if the following line has line
         * number line and starts with source string number
         * source-string-number". What that changes is `__LINE__` and
         * `__FILE__`; diagnostics still count the application's own lines,
         * which is what an info log's `0:<line>:` means and what dEQP reads.
         * Until 2026-09-23 the numbers were checked and then dropped, so
         * `#line 1` followed by `float(__LINE__)` gave the physical line. */
        let mut expanded = AVec::new();
        self.expand_into(body, &mut expanded, 0)?;
        let e = expanded.as_slice();
        if e.is_empty() {
            return Err(Error::new(name.line, "#line", "#line requires a line number"));
        }
        for t in e.iter().take(2) {
            if t.kind != Tok::IntLit {
                return Err(Error::new(t.line, t.text, "#line requires an integer"));
            }
        }
        if e.len() > 2 {
            return Err(Error::new(e[2].line, e[2].text, "extra tokens at the end of the directive"));
        }
        self.line_delta = i64::from(e[0].int) - (i64::from(name.line) + 1);
        if let Some(f) = e.get(1) {
            self.file = f.int;
        }
        Ok(())
    }

    /* ---- conditionals -------------------------------------------------- */

    fn d_if(&mut self, name: &Token<'a>, body: &'a [Token<'a>]) -> R<'a, ()> {
        let outer = self.active();
        /* A dead group's condition is not evaluated at all, so an undefined
         * identifier or a syntax error inside it is not an error -- that is
         * what makes `#ifdef NEVER / #if garbage / #endif / #endif` legal. */
        let v = if outer { self.eval_condition(name, body)? } else { false };
        self.cond.push(
            self.a,
            Cond { active: outer && v, taken: v, saw_else: false, outer_active: outer, line: name.line },
        )?;
        Ok(())
    }

    fn d_ifdef(&mut self, name: &Token<'a>, body: &'a [Token<'a>], negate: bool) -> R<'a, ()> {
        let outer = self.active();
        let mut v = false;
        if outer {
            let Some(id) = body.first() else {
                return Err(Error::new(name.line, name.text, "#ifdef requires a macro name"));
            };
            if id.kind != Tok::Ident {
                return Err(Error::new(id.line, id.text, "#ifdef requires a macro name"));
            }
            self.end_of_directive(body, 1)?;
            v = self.is_defined(id.text) != negate;
        }
        self.cond.push(
            self.a,
            Cond { active: outer && v, taken: v, saw_else: false, outer_active: outer, line: name.line },
        )?;
        Ok(())
    }

    fn d_elif(&mut self, name: &Token<'a>, body: &'a [Token<'a>]) -> R<'a, ()> {
        let Some(c) = self.cond.pop() else {
            return Err(Error::new(name.line, name.text, "#elif without #if"));
        };
        if c.saw_else {
            self.cond.push(self.a, c)?;
            return Err(Error::new(name.line, name.text, "#elif after #else"));
        }
        /* Only evaluate when the answer can still matter: if a branch was
         * already taken, or the whole chain is inside a dead group, the
         * expression is skipped exactly as a dead `#if`'s is. */
        let v = if c.outer_active && !c.taken { self.eval_condition(name, body)? } else { false };
        self.cond.push(self.a, Cond { active: c.outer_active && !c.taken && v, taken: c.taken || v, ..c })?;
        Ok(())
    }

    fn d_else(&mut self, name: &Token<'a>, body: &'a [Token<'a>]) -> R<'a, ()> {
        let Some(c) = self.cond.pop() else {
            return Err(Error::new(name.line, name.text, "#else without #if"));
        };
        if c.saw_else {
            self.cond.push(self.a, c)?;
            return Err(Error::new(name.line, name.text, "#else after #else"));
        }
        if c.outer_active {
            self.end_of_directive(body, 0)?;
        }
        self.cond.push(self.a, Cond { active: c.outer_active && !c.taken, taken: true, saw_else: true, ..c })?;
        Ok(())
    }

    fn d_endif(&mut self, name: &Token<'a>, body: &'a [Token<'a>]) -> R<'a, ()> {
        let Some(c) = self.cond.pop() else {
            return Err(Error::new(name.line, name.text, "#endif without #if"));
        };
        if c.outer_active {
            self.end_of_directive(body, 0)?;
        }
        Ok(())
    }

    /// Is `name` an object-like macro whose expansion is the one token
    /// `defined` (through a chain of such macros)? Then, in an `#if`, it
    /// IS the operator, and the name after it is its operand, unexpanded:
    /// `#define AAA defined` makes `#if AAA AAA` ask whether AAA is defined
    /// (dEQP-GLES3 preprocessor.conditional_inclusion.defined_macro_defined_test
    /// expects true). Expanding the operand as well gave `defined defined`.
    fn expands_to_defined(&self, name: &str) -> bool {
        let mut n = name;
        for _ in 0..16 {
            let Some(m) = self.find_macro(n) else { return false };
            if m.function_like || m.body.len() != 1 || m.body[0].kind != Tok::Ident {
                return false;
            }
            n = m.body[0].text;
            if n == "defined" {
                return true;
            }
        }
        false
    }

    fn is_defined(&self, name: &str) -> bool {
        is_predefined(name) || self.find_macro(name).is_some()
    }

    fn find_macro(&self, name: &str) -> Option<Macro<'a>> {
        self.macros.as_slice().iter().find(|m| m.name == name).copied()
    }

    /// Evaluate an `#if` / `#elif` condition.
    fn eval_condition(&mut self, name: &Token<'a>, body: &'a [Token<'a>]) -> R<'a, bool> {
        if body.is_empty() {
            return Err(Error::new(name.line, name.text, "#if requires an expression"));
        }
        /* `defined X` is resolved before expansion, because X must not be
         * expanded first (section 3.4). */
        let folded = self.fold_defined(name, body)?;
        let mut expanded = AVec::new();
        self.expand_into(folded, &mut expanded, 0)?;
        /* Fold again: `#define AAA defined(BBB)` then `#if !AAA` puts a
         * `defined` into the expression that was not there to fold the first
         * time. C calls that undefined; dEQP's conditional_inclusion.basic_2
         * expects it to work, so it is folded where it now stands.
         *
         * The limit of this: if the OPERAND is itself a macro it has already
         * been expanded by the time the second fold sees it, so `#define BBB
         * 1` alongside `#define AAA defined(BBB)` leaves `defined(1)` and
         * fails. The one shape dEQP does pin -- a macro that IS `defined`,
         * followed by its operand (`#define AAA defined`, `#if AAA AAA`) --
         * is caught by the FIRST fold (`expands_to_defined`), before
         * anything is expanded. */
        let toks = self.fold_defined(name, expanded.into_slice())?;
        let mut ev = Eval { toks, i: 0, line: name.line };
        let v = ev.expr(0, true)?;
        if ev.i < toks.len() {
            let t = toks[ev.i];
            return Err(Error::new(t.line, t.text, "extra tokens at the end of the directive"));
        }
        Ok(v != 0)
    }

    /// Replace every `defined X` / `defined ( X )` in `toks` with 1 or 0.
    ///
    /// The operand is never macro-expanded, which is why this runs before
    /// expansion -- and again after it, for a `defined` that expansion
    /// produced.
    fn fold_defined(&mut self, name: &Token<'a>, toks: &'a [Token<'a>]) -> R<'a, &'a [Token<'a>]> {
        if !toks.iter().any(|t| t.kind == Tok::Ident && (t.text == "defined" || self.expands_to_defined(t.text))) {
            return Ok(toks);
        }
        let mut out: AVec<Token<'a>> = AVec::with_capacity(self.a, toks.len())?;
        let mut i = 0usize;
        while i < toks.len() {
            let t = toks[i];
            if t.kind == Tok::Ident && (t.text == "defined" || self.expands_to_defined(t.text)) {
                let (id, next) = self.parse_defined_operand(name, toks, i + 1)?;
                let mut v = t;
                v.kind = Tok::IntLit;
                v.int = u32::from(self.is_defined(id));
                v.text = if v.int == 1 { "1" } else { "0" };
                out.push(self.a, v)?;
                i = next;
                continue;
            }
            out.push(self.a, t)?;
            i += 1;
        }
        Ok(out.into_slice())
    }

    /// `defined X` or `defined ( X )`; returns the name and the index after.
    fn parse_defined_operand(&self, name: &Token<'a>, body: &'a [Token<'a>], at: usize) -> R<'a, (&'a str, usize)> {
        let err = |t: Option<&Token<'a>>, msg: &'static str| match t {
            Some(t) => Error::new(t.line, t.text, msg),
            None => Error::new(name.line, name.text, msg),
        };
        match body.get(at) {
            Some(t) if t.kind == Tok::Ident => Ok((t.text, at + 1)),
            Some(t) if t.kind == Tok::LParen => {
                let Some(id) = body.get(at + 1).filter(|t| t.kind == Tok::Ident) else {
                    return Err(err(body.get(at + 1), "expected an identifier after 'defined('"));
                };
                match body.get(at + 2) {
                    Some(p) if p.kind == Tok::RParen => Ok((id.text, at + 3)),
                    other => Err(err(other, "expected ')' after 'defined('")),
                }
            }
            other => Err(err(other, "expected an identifier after 'defined'")),
        }
    }

    /* ---- macro expansion ----------------------------------------------- */

    fn expand_into(&mut self, toks: &[Token<'a>], out: &mut AVec<'a, Token<'a>>, depth: u32) -> R<'a, ()> {
        if depth > MAX_EXPANSION_DEPTH {
            let t = toks.first().copied().unwrap_or(self.toks[self.i]);
            return Err(Error::new(t.line, t.text, "macro expansion nested too deeply"));
        }
        let mut i = 0usize;
        /* A function-like macro name that an expansion ended on, when the
         * source token after that expansion (at `i`) is `(`: C99 6.10.3.4
         * rescans the replacement list together with the rest of the source,
         * so the name is invoked with those tokens (`#define G F`, `G(2)`).
         * The macro just expanded is no longer disabled by then. */
        let mut carried: Option<Token<'a>> = None;
        while carried.is_some() || i < toks.len() {
            let (t, lparen) = match carried.take() {
                Some(t) => (t, i),
                None => {
                    let t = toks[i];
                    if t.kind != Tok::Ident {
                        out.push(self.a, t)?;
                        i += 1;
                        continue;
                    }
                    if let Some(v) = self.predefined_value(&t) {
                        out.push(self.a, v)?;
                        i += 1;
                        continue;
                    }
                    (t, i + 1)
                }
            };
            let Some(mac) = self.find_macro(t.text) else {
                out.push(self.a, t)?;
                i += 1;
                continue;
            };
            if self.active_macros.as_slice().contains(&t.text) {
                /* ES 1.00 makes a recursive macro an error rather than a
                 * fixed point (dEQP recursion.recursion_1..5). */
                return Err(Error::new(t.line, t.text, "recursive macro definition"));
            }
            let start = out.len();
            if !mac.function_like {
                self.active_macros.push(self.a, mac.name)?;
                let outer = self.invocation_line;
                self.invocation_line = outer.or(Some(t.line));
                let r = self.expand_into(mac.body, out, depth + 1);
                self.invocation_line = outer;
                self.active_macros.pop();
                r?;
                i += 1;
                carried = self.tail_name(toks, i, out, start);
                continue;
            }
            /* A function-like macro name not followed by '(' is an ordinary
             * identifier (section 3.4), not an error. */
            if !toks.get(lparen).is_some_and(|t| t.kind == Tok::LParen) {
                out.push(self.a, t)?;
                i += 1;
                continue;
            }
            let (args, next) = self.collect_args(&t, toks, lparen)?;
            if args.len() != mac.params.len() {
                return Err(Error::new(t.line, t.text, "wrong number of arguments to a macro"));
            }
            let body = self.substitute(&mac, args, depth)?;
            self.active_macros.push(self.a, mac.name)?;
            /* The invocation ends on its `)`, the token before `next`. */
            let outer = self.invocation_line;
            self.invocation_line = outer.or(Some(toks[next - 1].line));
            let r = self.expand_into(body, out, depth + 1);
            self.invocation_line = outer;
            self.active_macros.pop();
            r?;
            i = next;
            carried = self.tail_name(toks, i, out, start);
        }
        Ok(())
    }

    /// If the expansion that began at `out[start]` ended on the name of a
    /// function-like macro, and `toks[next]` is `(`, take that name back out
    /// of `out` so the caller invokes it with the following source tokens.
    /// Only a name this expansion produced counts: an expansion of no tokens
    /// leaves what came before it alone (`F E (2)` with an empty `E`).
    fn tail_name(
        &self,
        toks: &[Token<'a>],
        next: usize,
        out: &mut AVec<'a, Token<'a>>,
        start: usize,
    ) -> Option<Token<'a>> {
        if out.len() <= start || !toks.get(next).is_some_and(|t| t.kind == Tok::LParen) {
            return None;
        }
        let last = *out.as_slice().last()?;
        if last.kind != Tok::Ident || self.active_macros.as_slice().contains(&last.text) {
            return None;
        }
        if !self.find_macro(last.text).is_some_and(|m| m.function_like) {
            return None;
        }
        out.pop();
        Some(last)
    }

    /// `__LINE__`, `__FILE__`, `__VERSION__`, `GL_ES` and
    /// `GL_FRAGMENT_PRECISION_HIGH` (when [`is_predefined`]) as tokens.
    fn predefined_value(&self, t: &Token<'a>) -> Option<Token<'a>> {
        let v = match t.text {
            "__LINE__" => {
                let line = self.invocation_line.unwrap_or(t.line);
                u32::try_from(i64::from(line) + self.line_delta).unwrap_or(0)
            }
            /* One source string, as glShaderSource concatenated it, until a
             * `#line` names another. */
            "__FILE__" => self.file,
            "__VERSION__" => self.version,
            "GL_ES" => 1,
            FRAGMENT_PRECISION_HIGH if is_predefined(FRAGMENT_PRECISION_HIGH) => 1,
            _ => return None,
        };
        let mut out = *t;
        out.kind = Tok::IntLit;
        out.int = v;
        Some(out)
    }

    /// The arguments of a function-like macro invocation whose `(` is at
    /// `lparen`; returns them and the index after the `)`.
    #[allow(clippy::type_complexity)]
    fn collect_args(
        &self,
        name: &Token<'a>,
        toks: &[Token<'a>],
        lparen: usize,
    ) -> R<'a, (&'a [&'a [Token<'a>]], usize)> {
        let mut args: AVec<&'a [Token<'a>]> = AVec::new();
        let mut depth = 0i32;
        let mut start = lparen + 1;
        let mut i = lparen;
        loop {
            let Some(t) = toks.get(i) else {
                return Err(Error::new(name.line, name.text, "missing ')' in a macro invocation"));
            };
            match t.kind {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        let last = self.a.alloc_copy(&toks[start..i])?;
                        /* `A()` is one empty argument, which is how a macro
                         * of no parameters is called; a macro of one
                         * parameter called that way gets an empty one. */
                        if !(args.is_empty() && last.is_empty()) {
                            args.push(self.a, last)?;
                        }
                        return Ok((args.into_slice(), i + 1));
                    }
                }
                Tok::Comma if depth == 1 => {
                    args.push(self.a, self.a.alloc_copy(&toks[start..i])?)?;
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// The macro body with each parameter replaced by its expanded argument.
    fn substitute(&mut self, mac: &Macro<'a>, args: &'a [&'a [Token<'a>]], depth: u32) -> R<'a, &'a [Token<'a>]> {
        /* Arguments are expanded before substitution. ES 1.00 has neither
         * `#` nor `##`, so there is no case where they must not be. */
        let mut expanded: AVec<&'a [Token<'a>]> = AVec::with_capacity(self.a, args.len())?;
        for arg in args {
            let mut e = AVec::new();
            self.expand_into(arg, &mut e, depth + 1)?;
            expanded.push(self.a, e.into_slice())?;
        }
        let expanded = expanded.into_slice();
        let mut out: AVec<Token<'a>> = AVec::with_capacity(self.a, mac.body.len())?;
        for t in mac.body {
            let param = if t.kind == Tok::Ident { mac.params.iter().position(|p| *p == t.text) } else { None };
            match param {
                Some(n) => out.extend_from_slice(self.a, expanded[n])?,
                None => out.push(self.a, *t)?,
            }
        }
        Ok(out.into_slice())
    }
}

/// Are two `#define`s the same definition?
///
/// C99 6.10.3p2: the replacement lists must consist of the same tokens with
/// white space in the same places, all white space being equivalent. dEQP's
/// `invalid_object_whitespace` (`(AAA - 1.0)` then `(AAA- 1.0)`) fails on the
/// white space alone, and `redefine_object_comment` passes with a different
/// comment in the same gap, so both halves of that sentence are load-bearing.
fn same_definition(a: &Macro<'_>, b: &Macro<'_>) -> bool {
    if a.function_like != b.function_like || a.params.len() != b.params.len() {
        return false;
    }
    if a.params.iter().zip(b.params).any(|(x, y)| x != y) {
        return false;
    }
    if a.body.len() != b.body.len() {
        return false;
    }
    a.body.iter().zip(b.body).enumerate().all(|(i, (x, y))| {
        x.kind == y.kind
            && x.text == y.text
            /* The first token's leading white space is the gap after the
             * macro name, which C does not count. */
            && (i == 0 || x.ws_before == y.ws_before)
    })
}

/* ---- the #if expression evaluator ---------------------------------------
 *
 * Signed 32-bit arithmetic with C's precedence, over the token list after
 * `defined` folding and macro expansion. dEQP's operator_precedence group is
 * 45 cases that do nothing but check this table, so the precedence is written
 * as a table rather than as a ladder of functions.
 */

struct Eval<'a> {
    toks: &'a [Token<'a>],
    i: usize,
    line: u32,
}

/// Binding power of a binary operator, or 0 for anything that is not one.
fn binding_power(t: Tok) -> u8 {
    match t {
        Tok::OrOr => 1,
        Tok::AndAnd => 2,
        Tok::Pipe => 3,
        Tok::Caret => 4,
        Tok::Amp => 5,
        Tok::EqEq | Tok::NotEq => 6,
        Tok::Lt | Tok::Gt | Tok::Le | Tok::Ge => 7,
        Tok::Shl | Tok::Shr => 8,
        Tok::Plus | Tok::Minus => 9,
        Tok::Star | Tok::Slash | Tok::Percent => 10,
        _ => 0,
    }
}

impl<'a> Eval<'a> {
    fn peek(&self) -> Token<'a> {
        self.toks.get(self.i).copied().unwrap_or(Token {
            kind: Tok::Eof,
            text: "",
            line: self.line,
            bol: false,
            ws_before: false,
            int: 0,
            float: 0.0,
            es3: false,
        })
    }

    /// Precedence climbing. `live` is false inside the arm of a `&&` or `||`
    /// that has already been decided: the tokens are still parsed, so a
    /// syntax error there is still an error, but an undefined identifier or a
    /// division by zero is not.
    fn expr(&mut self, min_bp: u8, live: bool) -> R<'a, i32> {
        let mut lhs = self.unary(live)?;
        loop {
            let op = self.peek();
            let bp = binding_power(op.kind);
            if bp == 0 || bp < min_bp {
                return Ok(lhs);
            }
            self.i += 1;
            /* Short circuit: `#if 1 || AAA` must compile although AAA is
             * undefined, and `#if A && B` must not (dEQP
             * undefined_identifiers). */
            let rhs_live = live
                && match op.kind {
                    Tok::OrOr => lhs == 0,
                    Tok::AndAnd => lhs != 0,
                    _ => true,
                };
            let rhs = self.expr(bp + 1, rhs_live)?;
            lhs = self.apply(op, lhs, rhs, live)?;
        }
    }

    fn apply(&self, op: Token<'a>, l: i32, r: i32, live: bool) -> R<'a, i32> {
        let div = |f: fn(i32, i32) -> i32| -> R<'a, i32> {
            if r == 0 {
                if !live {
                    return Ok(0);
                }
                return Err(Error::new(op.line, op.text, "division by zero in a preprocessor expression"));
            }
            Ok(f(l, r))
        };
        Ok(match op.kind {
            Tok::OrOr => i32::from(l != 0 || r != 0),
            Tok::AndAnd => i32::from(l != 0 && r != 0),
            Tok::Pipe => l | r,
            Tok::Caret => l ^ r,
            Tok::Amp => l & r,
            Tok::EqEq => i32::from(l == r),
            Tok::NotEq => i32::from(l != r),
            Tok::Lt => i32::from(l < r),
            Tok::Gt => i32::from(l > r),
            Tok::Le => i32::from(l <= r),
            Tok::Ge => i32::from(l >= r),
            Tok::Shl => l.wrapping_shl(r as u32),
            Tok::Shr => l.wrapping_shr(r as u32),
            Tok::Plus => l.wrapping_add(r),
            Tok::Minus => l.wrapping_sub(r),
            Tok::Star => l.wrapping_mul(r),
            Tok::Slash => div(i32::wrapping_div)?,
            Tok::Percent => div(i32::wrapping_rem)?,
            _ => return Err(Error::new(op.line, op.text, "invalid operator in a preprocessor expression")),
        })
    }

    fn unary(&mut self, live: bool) -> R<'a, i32> {
        let t = self.peek();
        match t.kind {
            Tok::Plus => {
                self.i += 1;
                self.unary(live)
            }
            Tok::Minus => {
                self.i += 1;
                Ok(self.unary(live)?.wrapping_neg())
            }
            Tok::Not => {
                self.i += 1;
                Ok(i32::from(self.unary(live)? == 0))
            }
            Tok::Tilde => {
                self.i += 1;
                Ok(!self.unary(live)?)
            }
            Tok::LParen => {
                self.i += 1;
                let v = self.expr(0, live)?;
                if self.peek().kind != Tok::RParen {
                    let p = self.peek();
                    return Err(Error::new(p.line, p.text, "expected ')' in a preprocessor expression"));
                }
                self.i += 1;
                Ok(v)
            }
            Tok::IntLit => {
                self.i += 1;
                Ok(t.int as i32)
            }
            Tok::FloatLit => Err(Error::new(t.line, t.text, "a preprocessor expression must be an integer expression")),
            Tok::Ident => {
                self.i += 1;
                if live {
                    /* GLSL's boolean keywords are not preprocessor tokens
                     * either: in `#if`, `true` is just an undefined name. */
                    Err(Error::new(t.line, t.text, "undefined identifier in a preprocessor expression"))
                } else {
                    Ok(0)
                }
            }
            Tok::Eof => Err(Error::new(t.line, "", "a preprocessor expression is missing")),
            _ => Err(Error::new(t.line, t.text, "invalid token in a preprocessor expression")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Preprocess `src` and return the resulting token spellings.
    fn pp<'a>(buf: &'a mut [u8], src: &'a str) -> R<'a, std::vec::Vec<&'a str>> {
        let a = Arena::new(buf);
        let r = preprocess(&a, src)?;
        Ok(r.tokens.iter().filter(|t| t.kind != Tok::Eof).map(|t| t.text).collect())
    }

    fn ok(src: &str) -> std::vec::Vec<std::string::String> {
        let mut buf = [0u8; 1 << 16];
        match pp(&mut buf, src) {
            Ok(v) => v.iter().map(|s| s.to_string()).collect(),
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    fn fails(src: &str) -> &'static str {
        let mut buf = [0u8; 1 << 16];
        match pp(&mut buf, src) {
            Ok(v) => panic!("expected a compile failure, got {v:?}"),
            Err(e) => e.msg,
        }
    }

    /// Section 3.4: "All macro names prefixed with 'GL_' ('GL' followed by a
    /// single underscore) are also reserved."
    ///
    /// dEQP `invalid_redefinitions.invalid_identifier_2`. Only this half of
    /// the sentence is enforced -- see the module note on `__`, where dEQP
    /// contradicts the specification with a must-PASS case and the must-pass
    /// side wins.
    #[test]
    fn a_macro_name_cannot_be_prefixed_with_gl_() {
        assert_eq!(
            fails(
                "#define GL_VALUE 1.0
float x = GL_VALUE;"
            ),
            "'GL_' is a reserved macro prefix"
        );
        assert_eq!(
            fails(
                "#define GL_A(x) x
"
            ),
            "'GL_' is a reserved macro prefix",
            "function-like too"
        );
        /* `GL_ES` is predefined, so it is caught by the earlier rule with
         * the message that says so. */
        assert_eq!(
            fails(
                "#define GL_ES 2
"
            ),
            "cannot redefine a predefined macro"
        );
        /* Not reserved: a name merely CONTAINING it, a lone `GL`, and the
         * double underscore dEQP's `identifier_with_double_underscore`
         * requires to build. */
        assert_eq!(
            ok("#define MY_GL_VALUE 1
int x = MY_GL_VALUE;"),
            ["int", "x", "=", "1", ";"]
        );
        assert_eq!(
            ok("#define GL 1
int x = GL;"),
            ["int", "x", "=", "1", ";"]
        );
        assert_eq!(
            ok("#define __VALUE__ 1
int x = __VALUE__;"),
            ["int", "x", "=", "1", ";"]
        );
    }

    /// Section 3.4: "the extension directives must occur before any
    /// non-preprocessor tokens."
    ///
    /// dEQP `extensions.after_non_preprocessing_tokens`, which puts the
    /// second `#extension` inside `main` -- there is no scope smaller than
    /// the compilation unit for it to apply to.
    #[test]
    fn an_extension_directive_comes_before_any_other_token() {
        const AFTER: &str = "#extension must come before any other token";
        assert_eq!(
            fails(
                "precision mediump float;
#extension all : warn
"
            ),
            AFTER
        );
        assert_eq!(
            fails(
                "void main() {
#extension all : disable
}"
            ),
            AFTER,
            "inside a function"
        );
        /* Before any other token is where it belongs, and other directives
         * do not count as "other tokens". */
        assert_eq!(
            ok("#extension all : warn
int x = 1;"),
            ["int", "x", "=", "1", ";"]
        );
        assert_eq!(
            ok("#define A 1
#extension all : warn
int x = A;"),
            ["int", "x", "=", "1", ";"]
        );
    }

    #[test]
    fn an_object_macro_expands() {
        assert_eq!(ok("#define A 1\nint x = A;"), ["int", "x", "=", "1", ";"]);
    }

    #[test]
    fn a_function_macro_expands_with_its_arguments() {
        assert_eq!(
            ok("#define ADD(a,b) ((a)+(b))\nint x = ADD(1,2);"),
            ["int", "x", "=", "(", "(", "1", ")", "+", "(", "2", ")", ")", ";"]
        );
    }

    /// `#define A(x)` is function-like and `#define A (x)` is an object-like
    /// macro whose body is `(x)`. The only difference in the text is the
    /// space, which is why the lexer carries `ws_before`.
    #[test]
    fn a_space_before_the_paren_makes_an_object_macro() {
        assert_eq!(ok("#define A (x)\nint y = A;"), ["int", "y", "=", "(", "x", ")", ";"]);
        assert_eq!(ok("#define A(x) x\nint y = A(3);"), ["int", "y", "=", "3", ";"]);
    }

    /// dEQP object_redefinitions: identical is fine, different white space is
    /// not, a different comment in the same gap is.
    #[test]
    fn redefinition_must_be_identical_down_to_the_white_space() {
        ok("#define V (A - 1.0)\n#define V (A - 1.0)\nfloat x;");
        assert_eq!(
            fails("#define V (A - 1.0)\n#define V (A- 1.0)\nfloat x;"),
            "macro redefined with a different definition"
        );
        assert_eq!(
            fails("#define V (A - 1.0)\n#define V (B - 1.0)\nfloat x;"),
            "macro redefined with a different definition"
        );
        ok("#define V /* one */ (A - 1.0)\n#define V /* two */ (A - 1.0)\nfloat x;");
    }

    /// dEQP recursion.recursion_1: ES 1.00 makes this an error, where C's
    /// blue-paint rule would accept it.
    #[test]
    fn a_recursive_macro_is_an_error() {
        assert_eq!(fails("#define AAA AAA\nfloat x = AAA;"), "recursive macro definition");
        assert_eq!(fails("#define A B\n#define B A\nfloat x = A;"), "recursive macro definition");
    }

    #[test]
    fn conditionals_select_the_right_branch() {
        assert_eq!(ok("#if 1\nint a;\n#else\nint b;\n#endif"), ["int", "a", ";"]);
        assert_eq!(ok("#if 0\nint a;\n#else\nint b;\n#endif"), ["int", "b", ";"]);
        assert_eq!(ok("#if 0\nint a;\n#elif 1\nint b;\n#else\nint c;\n#endif"), ["int", "b", ";"]);
        assert_eq!(ok("#define X\n#ifdef X\nint a;\n#endif"), ["int", "a", ";"]);
        assert_eq!(ok("#ifndef X\nint a;\n#endif"), ["int", "a", ";"]);
        assert_eq!(ok("#if defined(GL_ES)\nint a;\n#endif"), ["int", "a", ";"], "GL_ES is predefined");
    }

    /// Section 4.5.4 on a system with fragment highp (every GPU here,
    /// backend::FRAGMENT_HIGHP): GL_FRAGMENT_PRECISION_HIGH is defined, is 1,
    /// takes the `#ifdef` branch, and is predefined like GL_ES (no `#define`,
    /// no `#undef`). glmark2's shaders depend on it: jellyfish asks for highp
    /// only under it, and conditionals/function/loop define HIGHP_OR_DEFAULT
    /// twice, identically only when it is defined (driver/docs/glmark2.md).
    #[test]
    fn fragment_precision_high_is_predefined_to_one() {
        if !crate::backend::FRAGMENT_HIGHP {
            /* a system without fragment highp: the macro is not defined */
            assert_eq!(ok("#ifdef GL_FRAGMENT_PRECISION_HIGH\nint a;\n#else\nint b;\n#endif"), ["int", "b", ";"]);
            return;
        }
        assert_eq!(
            ok("precision mediump float;\n#ifdef GL_FRAGMENT_PRECISION_HIGH\nint a;\n#else\nint b;\n#endif"),
            ["precision", "mediump", "float", ";", "int", "a", ";"]
        );
        assert_eq!(ok("#if GL_FRAGMENT_PRECISION_HIGH == 1\nint a;\n#endif"), ["int", "a", ";"]);
        assert_eq!(ok("#if defined(GL_FRAGMENT_PRECISION_HIGH)\nint a;\n#endif"), ["int", "a", ";"]);
        assert_eq!(ints_of("int v = GL_FRAGMENT_PRECISION_HIGH;"), [1], "an integer 1, as GL_ES");
        // glmark2's shader-source.cc preamble, twice (once fragment-typed,
        // once untyped), as conditionals.frag gets it: one definition both times
        let pre_fs = "#if defined(GL_ES) && defined(GL_FRAGMENT_PRECISION_HIGH)\n#define HIGHP_OR_DEFAULT highp\n#else\n#define HIGHP_OR_DEFAULT\n#endif\n";
        let pre_any = "#if defined(GL_ES)\n#define HIGHP_OR_DEFAULT highp\n#else\n#define HIGHP_OR_DEFAULT\n#endif\n";
        let src = std::format!("{pre_fs}{pre_any}HIGHP_OR_DEFAULT float x;");
        assert_eq!(ok(&src), ["highp", "float", "x", ";"]);
        assert_eq!(fails("#undef GL_FRAGMENT_PRECISION_HIGH"), "cannot undefine a predefined macro");
        assert_eq!(fails("#define GL_FRAGMENT_PRECISION_HIGH 0"), "cannot redefine a predefined macro");
    }

    /// The condition of a dead group is never evaluated, so the nested `#if`
    /// here is not an error although its expression is nonsense. Evaluating
    /// it eagerly is the natural mistake, and it breaks real shaders.
    #[test]
    fn a_dead_groups_condition_is_not_evaluated() {
        assert_eq!(ok("#if 0\n#if !!! garbage\nint a;\n#endif\n#endif\nint b;"), ["int", "b", ";"]);
        assert_eq!(ok("#if 1\nint b;\n#elif UNDEFINED_THING\nint a;\n#endif"), ["int", "b", ";"]);
    }

    /// dEQP undefined_identifiers: `1 || AAA` passes and `A && B` fails, so
    /// the rule is "an undefined identifier that is actually evaluated".
    #[test]
    fn an_undefined_identifier_is_an_error_only_where_it_is_evaluated() {
        assert_eq!(ok("#if 1 || AAA\nint a;\n#endif"), ["int", "a", ";"]);
        assert_eq!(ok("#if 0 && AAA\nint a;\n#else\nint b;\n#endif"), ["int", "b", ";"]);
        assert_eq!(fails("#if A && B\nint a;\n#endif"), "undefined identifier in a preprocessor expression");
        assert_eq!(fails("#if 1 - CCC\nint a;\n#endif"), "undefined identifier in a preprocessor expression");
    }

    /// dEQP conditional_inclusion.basic_2: `#define AAA defined(BBB)` then
    /// `#if !AAA`. The `defined` is not in the directive's own tokens, so
    /// folding only before expansion leaves it as an undefined identifier.
    #[test]
    fn a_defined_that_a_macro_produced_is_still_folded() {
        assert_eq!(ok("#define AAA defined(BBB)\n#if !AAA\nint a;\n#endif"), ["int", "a", ";"]);
        assert_eq!(ok("#define AAA defined BBB\n#if !AAA\nint a;\n#endif"), ["int", "a", ";"]);
    }

    /// dEQP pragmas.pragma_unrecognized_token is a `#pragma` of non-ASCII
    /// bytes and expects the shader to BUILD; the same bytes in live code are
    /// an error. The lexer makes them a token so that both can be true.
    #[test]
    fn a_pragma_may_hold_characters_live_code_may_not() {
        assert_eq!(ok("#pragma \u{00a4}\u{00a4}\u{00bd}\nint a;"), ["int", "a", ";"]);
        assert_eq!(fails("int \u{00a4} = 1;"), "illegal character");
        assert_eq!(fails("int a = 1 @ 2;"), "illegal character");
    }

    /// dEQP function_redefinition_3 and redefine_object_multiline_comment: a
    /// comment that spans lines is one space, so the directive continues past
    /// it. Ending the directive there truncates the replacement list, and the
    /// next identical definition then looks like a redefinition.
    #[test]
    fn a_directive_survives_a_comment_that_spans_lines() {
        assert_eq!(
            ok("#define F(a,b) (a  +b)\n#define F(a,b)(a    /* one\n  two */ +b)\nfloat x = F(1.0,2.0);"),
            ["float", "x", "=", "(", "1.0", "+", "2.0", ")", ";"],
            "the second definition is function-like with the same body, so it is benign"
        );
        ok("#define R /* one\n two */ (A)\n#define R /* three */ (A)\nfloat x;");
    }

    #[test]
    fn the_expression_grammar_has_cs_precedence() {
        let cases: [(&str, bool); 12] = [
            ("1 + 2 * 3 == 7", true),
            ("(1 + 2) * 3 == 9", true),
            ("1 << 3 == 8", true),
            ("16 >> 2 == 4", true),
            ("7 % 4 == 3", true),
            ("-3 / 2 == -1", true),
            ("~0 == -1", true),
            ("!0 == 1", true),
            ("1 | 2 ^ 3 & 1 == 1", true),
            ("2 - 3 < 0", true),
            ("1 && 0 || 1", true),
            ("0 || 0", false),
        ];
        for (expr, want) in cases {
            let mut buf = [0u8; 1 << 16];
            let src = std::format!("#if {expr}\nint a;\n#endif");
            let a = Arena::new(&mut buf);
            let r = preprocess(&a, &src).expect("preprocess");
            let taken = r.tokens.iter().any(|t| t.text == "a");
            assert_eq!(taken, want, "#if {expr}");
        }
    }

    #[test]
    fn a_float_in_a_condition_is_an_error() {
        assert_eq!(fails("#if 1.0\nint a;\n#endif"), "a preprocessor expression must be an integer expression");
    }

    #[test]
    fn unbalanced_conditionals_are_errors() {
        assert_eq!(fails("#if 1\nint a;"), "#if without #endif");
        assert_eq!(fails("#endif\nint a;"), "#endif without #if");
        assert_eq!(fails("#else\nint a;"), "#else without #if");
        assert_eq!(fails("#elif 1\nint a;"), "#elif without #if");
        assert_eq!(fails("#if 1\n#else\n#else\n#endif"), "#else after #else");
        assert_eq!(fails("#if 1\n#else\n#elif 1\n#endif"), "#elif after #else");
    }

    /// dEQP invalid_conditionals.tokens_after_endif and its neighbours.
    #[test]
    fn tokens_after_a_conditional_directive_are_errors() {
        assert_eq!(fails("#if 1\n#endif garbage\nint a;"), "extra tokens at the end of the directive");
        assert_eq!(fails("#if 1\n#else garbage\n#endif"), "extra tokens at the end of the directive");
        assert_eq!(fails("#ifdef A garbage\n#endif"), "extra tokens at the end of the directive");
    }

    /// A macro that expands to `defined` is the operator in an `#if`, and
    /// the name after it is its operand, unexpanded (dEQP-GLES3
    /// preprocessor.conditional_inclusion.defined_macro_defined_test).
    ///
    /// Checked: without `expands_to_defined` in `fold_defined` this reads 0.
    #[test]
    fn a_macro_that_is_defined_takes_its_operand_unexpanded() {
        assert_eq!(ints_of("#define AAA defined\n#if AAA AAA\nint x = 1;\n#else\nint x = 0;\n#endif\n"), [1]);
        assert_eq!(ints_of("#define AAA defined\n#if AAA BBB\nint x = 1;\n#else\nint x = 0;\n#endif\n"), [0]);
    }

    #[test]
    fn predefined_macros_have_their_values() {
        let mut buf = [0u8; 1 << 16];
        let a = Arena::new(&mut buf);
        let r = preprocess(&a, "int v = __VERSION__;\nint e = GL_ES;\nint l = __LINE__;").expect("pp");
        let ints: std::vec::Vec<u32> = r.tokens.iter().filter(|t| t.kind == Tok::IntLit).map(|t| t.int).collect();
        assert_eq!(ints, [100, 1, 3], "__VERSION__, GL_ES, and __LINE__ on line 3");
    }

    fn ints_of(src: &str) -> std::vec::Vec<u32> {
        let mut buf = [0u8; 1 << 16];
        let a = Arena::new(&mut buf);
        let r = preprocess(&a, src).expect("pp");
        r.tokens.iter().filter(|t| t.kind == Tok::IntLit).map(|t| t.int).collect()
    }

    /// Section 3.4: after `#line line source-string-number` "the following
    /// line has line number line", and `__FILE__` is that string number.
    /// Both were checked and then dropped until 2026-09-23 (dEQP
    /// preprocessor.builtin.line and line_and_file read the physical line).
    ///
    /// Revert check: leave `line_delta` at 0 in `d_line` and the first
    /// assertion reads 2.
    #[test]
    fn line_renumbers_what_follows_it() {
        assert_eq!(ints_of("#line 1\nint l = __LINE__;"), [1]);
        assert_eq!(
            ints_of("int a = __LINE__;\n#line 234 10\nint l = __LINE__, f = __FILE__;\nint m = __LINE__;"),
            [1, 234, 10, 235]
        );
    }

    /// `__LINE__` in a replacement list is the line the invocation ENDS on;
    /// in an argument it is the argument's own line, because arguments are
    /// replaced first (dEQP preprocessor.predefined_macros.line_2).
    #[test]
    fn line_in_a_macro_is_where_the_invocation_ends() {
        assert_eq!(ints_of("#define B __LINE__\n#define A(x) B, x\nA(__LINE__\n\n)"), [5, 3]);
    }

    /// ES 3.00 lets `defined` be defined (dEQP-GLES3
    /// conditional_inclusion.define_defined and _outside_if): in an `#if` a
    /// macro that expands to `defined` is still the operator, and outside
    /// one the name expands like any macro.
    ///
    /// Revert check: refuse `#define defined` in ES 3.00 as in ES 1.00 and
    /// both shapes fail with "cannot redefine a predefined macro".
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn es300_may_define_defined() {
        let v = ok("#version 300 es\n#define CCC 1\n#define defined BBB\n#define AAA defined\n\
                    #if AAA CCC\nint yes;\n#else\nint no;\n#endif\n");
        assert!(v.contains(&"yes".to_string()) && !v.contains(&"no".to_string()), "{v:?}");
        let v = ok("#version 300 es\n#define defined 5\n#define AAA defined\nint x = AAA;\n");
        assert!(v.contains(&"5".to_string()), "{v:?}");
    }

    #[test]
    fn a_predefined_macro_cannot_be_defined_or_undefined() {
        assert_eq!(fails("#undef __LINE__\nint a;"), "cannot undefine a predefined macro");
        assert_eq!(fails("#define GL_ES 2\nint a;"), "cannot redefine a predefined macro");
        assert_eq!(fails("#define defined 1\nint a;"), "cannot redefine a predefined macro");
    }

    #[test]
    fn version_must_come_first() {
        ok("#version 100\nint a;");
        assert_eq!(fails("int a;\n#version 100\n"), "#version must be the first directive in the shader");
        assert_eq!(fails("#define A 1\n#version 100\n"), "#version must be the first directive in the shader");
        /* A comment before it is fine: only tokens count. */
        ok("/* header */\n#version 100\nint a;");
    }

    #[test]
    fn an_unsupported_version_is_rejected() {
        /* Desktop GLSL, which every source in the offline tables written
         * with explicit locations is: a CORRECT refusal by an ES compiler,
         * and it now says which kind of source it is looking at. Eighteen
         * sources under tools/shaders are `#version 450`, and the
         * twin-draw survey carried them as an unattributed bucket until
         * this message named them. */
        assert_eq!(fails("#version 450\nint a;"), "desktop GLSL source; this is an OpenGL ES compiler");
        assert_eq!(fails("#version 110\nint a;"), "desktop GLSL source; this is an OpenGL ES compiler");
        /* An ES number with `es` left off is a typo, not desktop: there is
         * no desktop GLSL 300 to have meant. */
        assert_eq!(fails("#version 300\nint a;"), "an ES version number needs 'es' after it");
        assert_eq!(fails("#version 310\nint a;"), "an ES version number needs 'es' after it");
        /* And a number that is neither keeps the generic sentence. */
        assert_eq!(fails("#version 100 es\nint a;"), "unsupported shading language version");
    }

    #[test]
    fn undef_removes_a_macro_and_undef_of_nothing_is_fine() {
        assert_eq!(ok("#define A 1\n#undef A\nint x = A;"), ["int", "x", "=", "A", ";"]);
        ok("#undef NEVER_DEFINED\nint a;");
    }

    #[test]
    fn a_function_macro_name_without_a_paren_is_an_identifier() {
        assert_eq!(ok("#define F(x) x\nint F;"), ["int", "F", ";"]);
    }

    #[test]
    fn the_wrong_number_of_macro_arguments_is_an_error() {
        assert_eq!(fails("#define F(a,b) a\nint x = F(1);"), "wrong number of arguments to a macro");
        assert_eq!(fails("#define F(a) a\nint x = F(1,2);"), "wrong number of arguments to a macro");
        assert_eq!(fails("#define F(a) a\nint x = F(1;"), "missing ')' in a macro invocation");
    }

    #[test]
    fn a_duplicate_macro_parameter_is_an_error() {
        assert_eq!(fails("#define F(a,a) a\nint x;"), "duplicate macro parameter");
    }

    #[test]
    fn an_unknown_directive_is_an_error_but_a_null_directive_is_not() {
        assert_eq!(fails("#foo\nint a;"), "invalid preprocessor directive");
        assert_eq!(ok("#\nint a;"), ["int", "a", ";"]);
    }

    /// An unknown directive inside a dead group is not an error: only the
    /// conditional directives are interpreted there.
    #[test]
    fn a_dead_group_hides_an_unknown_directive() {
        assert_eq!(ok("#if 0\n#foo\n#endif\nint a;"), ["int", "a", ";"]);
    }

    #[test]
    fn error_fails_with_its_own_message() {
        assert_eq!(fails("#error something went wrong\nint a;"), "#error");
    }

    /// `require` is decided by `src/gl/state.rs`'s advertised list, not by a
    /// copy here, so adding a name there is all it takes for the compiler to
    /// start accepting shaders that require it.
    #[test]
    fn extension_directives_are_checked() {
        ok("#extension all : disable\nint a;");
        ok("#extension GL_OES_standard_derivatives : warn\nint a;");
        assert_eq!(fails("#extension all : require\nint a;"), "'all' cannot be required or enabled");
        ok("#extension GL_OES_EGL_image : require\nint a;");
        assert_eq!(fails("#extension GL_NV_invented_here : require\nint a;"), "extension is not supported");
        assert_eq!(fails("#extension : require\nint a;"), "#extension requires an extension name");
        assert_eq!(fails("#extension GL_FOO require\nint a;"), "expected ':' after the extension name");
        assert_eq!(fails("#extension GL_FOO : banana\nint a;"), "invalid #extension behavior");
    }

    #[test]
    fn an_unrecognised_pragma_is_ignored() {
        assert_eq!(ok("#pragma whatever(on)\nint a;"), ["int", "a", ";"]);
    }

    /// A `#` that is not the first token of a line is not a directive, and
    /// ES 1.00 has no other use for one -- there is no `##` either.
    #[test]
    fn a_hash_in_the_middle_of_a_line_is_an_error() {
        assert_eq!(fails("int a # b;"), "'#' is only allowed at the start of a line");
        assert_eq!(fails("#define A 1\nint x = A ## 2;"), "'#' is only allowed at the start of a line");
    }

    /// A function-like invocation may span lines: the tokens are buffered
    /// until a directive or the end of the file, not line by line.
    #[test]
    fn a_macro_invocation_may_span_lines() {
        assert_eq!(ok("#define F(a,b) a b\nint F(x,\n      y);"), ["int", "x", "y", ";"]);
    }

    #[test]
    fn nested_macros_expand() {
        assert_eq!(ok("#define A 1\n#define B A\n#define C B\nint x = C;"), ["int", "x", "=", "1", ";"]);
        assert_eq!(
            ok("#define VAL2 2.0\n#define VAL1 1.0\n#define RES1 (VAL2/VAL1)\nfloat x = RES1;"),
            ["float", "x", "=", "(", "2.0", "/", "1.0", ")", ";"]
        );
    }

    /// A name with a double underscore is reserved by the specification, but
    /// dEQP's basic.identifier_with_double_underscore defines one and expects
    /// the shader to build, so rejecting it would fail a must-pass case.
    #[test]
    fn a_double_underscore_name_is_allowed() {
        assert_eq!(ok("#define __VALUE__ 1\nint x = __VALUE__;"), ["int", "x", "=", "1", ";"]);
    }

    /// A dialect out of scope is recognised for what it is and refused for
    /// that reason, at its version line. The point of the test is the reason:
    /// `1.0f` on the next line lexes only under ES 3.00 rules, so if the
    /// refusal ever moved after tokenizing, the message would become a lexical
    /// complaint about the literal and the dialect would go unnamed.
    #[test]
    fn out_of_scope_dialects_are_refused_by_name_at_the_version_line() {
        for (src, want) in [
            (
                "#version 310 es\nfloat x = 1.0f;\n",
                "GLSL ES 3.10 source; this compiler implements GLSL ES 1.00 and 3.00",
            ),
            (
                "#version 320 es\nfloat x = 1.0f;\n",
                "GLSL ES 3.20 source; this compiler implements GLSL ES 1.00 and 3.00",
            ),
        ] {
            assert_eq!(fails(src), want);
        }
        assert_eq!(scan_version("#version 300 es\n").expect("read"), (300, Version::Es300));
    }

    /// GLSL ES 3.00 is preprocessed under its own rules -- here the `f`
    /// suffix, which is no token at all in 1.00 -- and `__VERSION__` is 300;
    /// except in a build that reports ES 2.0, which refuses it by name.
    #[test]
    fn es300_is_preprocessed_unless_the_build_reports_es2() {
        let src = "#version 300 es\nfloat x = 1.0f;\nint v = __VERSION__;\n";
        if cfg!(feature = "report-es2") {
            assert_eq!(fails(src), "GLSL ES 3.00 source; this build reports OpenGL ES 2.0");
        } else {
            assert_eq!(ok(src)[..5], ["float", "x", "=", "1.0f", ";"]);
            assert_eq!(ints_of(src), [300], "__VERSION__");
        }
    }

    #[test]
    fn the_scratch_buffer_running_out_says_so() {
        let mut buf = [0u8; 48];
        let a = Arena::new(&mut buf);
        let Err(e) = preprocess(&a, "#define A 1\nint x = A;") else {
            panic!("a 48-byte scratch buffer cannot hold this shader's tokens");
        };
        assert_eq!(e.msg, "compiler scratch buffer exhausted", "and not a language error");
    }
}
