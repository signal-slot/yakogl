// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES 1.00 tokens (the OpenGL ES Shading Language 1.00 specification,
//! section 3).
//!
//! The lexer is shared by the preprocessor and the parser, because in GLSL
//! they see the same tokens: a directive is an ordinary token sequence that
//! happens to start with `#` at the beginning of a line. Two flags on each
//! token carry what the preprocessor needs and the parser ignores:
//!
//! * [`Token::bol`] -- this token is the first on its logical line, so a `#`
//!   here starts a directive and a `#` anywhere else is an error.
//! * [`Token::ws_before`] -- whitespace (or a comment) separated this token
//!   from the one before it. `#define A(x)` is a function-like macro and
//!   `#define A (x)` is an object-like macro whose body is `(x)`; the only
//!   difference is this flag on the `(`.
//!
//! Three ES 1.00 rules that ES 3.00 and desktop GLSL relax, and that dEQP
//! checks, are enforced here rather than left to a later pass:
//!
//! 1. **No line continuation.** ES 1.00 section 3.1 says there is none; a
//!    backslash is simply not in the language's character set. ES 3.00 added
//!    it, so [`Lexer::new`] takes the version and only then honours it.
//! 2. **No floating-point suffix.** `1.0f` is ES 3.00; in ES 1.00 the `f`
//!    would glue an identifier to a constant, which is no token at all.
//! 3. **The source character set is ASCII** outside comments.

use super::arena::{AVec, Arena};
use super::{Error, R};

/// What a token is. Keywords are [`Tok::Ident`]; [`keyword`] classifies them,
/// because the preprocessor must not treat `float` differently from `foo`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Ident,
    /// An integer constant. The value is in [`Token::int`].
    IntLit,
    /// A floating-point constant. The value is in [`Token::float`].
    FloatLit,

    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Dot,
    Comma,
    Colon,
    Semi,
    Question,

    Plus,
    Minus,
    Star,
    Slash,
    Percent,

    Lt,
    Gt,
    Le,
    Ge,
    EqEq,
    NotEq,

    AndAnd,
    OrOr,
    XorXor,
    Not,

    Tilde,
    Amp,
    Pipe,
    Caret,
    Shl,
    Shr,

    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    AndAssign,
    OrAssign,
    XorAssign,
    ShlAssign,
    ShrAssign,

    Inc,
    Dec,

    /// `#`, meaningful to the preprocessor only.
    Hash,
    /// `##`, the paste operator. ES 1.00 section 3.4 gives the preprocessor
    /// no `##`, so using one is an error; the lexer still names it, so that
    /// the error can say what it saw.
    HashHash,

    /// A character that is in no GLSL token: a byte outside the source
    /// character set (section 3.1), or an ASCII one the language has no use
    /// for (`@`, `$`, a quote, a backslash outside ES 3.00).
    ///
    /// It is a token rather than an immediate error because a `#pragma` is
    /// allowed to hold anything at all -- dEQP's
    /// `pragmas.pragma_unrecognized_token` is a `#pragma` of non-ASCII bytes
    /// and expects the shader to BUILD. The preprocessor rejects an `Unknown`
    /// that reaches live code, which is where it is really wrong.
    Unknown,

    /// End of input. Always the last token, so a parser can look ahead
    /// without a bounds check.
    Eof,
}

/// One token.
///
/// `text` borrows the source, so a diagnostic can quote the exact spelling
/// with no copy and no allocation.
#[derive(Clone, Copy, Debug)]
pub struct Token<'a> {
    pub kind: Tok,
    pub text: &'a str,
    /// 1-based line of the first character.
    pub line: u32,
    /// First token on its logical line.
    pub bol: bool,
    /// Whitespace or a comment preceded this token.
    pub ws_before: bool,
    /// [`Tok::IntLit`]: the value. Held as `u32` because GLSL ES 1.00 has no
    /// negative literals -- `-1` is unary minus applied to `1` -- so the
    /// literal itself is never negative, and 0xffffffff must survive.
    pub int: u32,
    /// [`Tok::FloatLit`]: the value.
    pub float: f32,
    /// Lexed as GLSL ES 3.00 (`#version 300 es`): which words are keywords
    /// and which are reserved differs by version, and every later pass asks
    /// the token rather than being handed the version separately.
    pub es3: bool,
}

impl<'a> Token<'a> {
    fn new(kind: Tok, text: &'a str, line: u32, bol: bool, ws_before: bool) -> Token<'a> {
        Token { kind, text, line, bol, ws_before, int: 0, float: 0.0, es3: false }
    }

    /// The keyword this token is, if it is one.
    pub fn kw(&self) -> Option<Kw> {
        if self.kind == Tok::Ident {
            if self.es3 {
                keyword_es3(self.text)
            } else {
                keyword(self.text)
            }
        } else {
            None
        }
    }
}

/// The GLSL ES 1.00 keywords, plus the ones the specification *reserves* for
/// future use (section 3.6). A reserved word is a compile error where it is
/// used, which is a rule dEQP checks, so the lexer must be able to name them
/// rather than let them through as identifiers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kw {
    /* types */
    Void,
    Bool,
    Int,
    Float,
    Vec2,
    Vec3,
    Vec4,
    BVec2,
    BVec3,
    BVec4,
    IVec2,
    IVec3,
    IVec4,
    Mat2,
    Mat3,
    Mat4,
    Sampler2D,
    SamplerCube,
    Struct,

    /* qualifiers */
    Attribute,
    Const,
    Uniform,
    Varying,
    In,
    Out,
    InOut,
    Invariant,
    Precision,
    HighP,
    MediumP,
    LowP,

    /* statements */
    Break,
    Continue,
    Do,
    For,
    While,
    If,
    Else,
    Return,
    Discard,

    /* true / false are keywords in GLSL, not identifiers */
    True,
    False,

    /* GLSL ES 3.00 only (section 3.8 of that specification); in 1.00 each
     * is an identifier or, where 1.00 reserved it, `Reserved`. */
    Uint,
    UVec2,
    UVec3,
    UVec4,
    /// `matCxR`, C columns of R rows. `mat2x2` is `mat2`, and so on; those
    /// three spellings lex as [`Kw::Mat2`], [`Kw::Mat3`], [`Kw::Mat4`].
    MatCxR(u8, u8),
    Sampler3D,
    Sampler2DShadow,
    SamplerCubeShadow,
    Sampler2DArray,
    Sampler2DArrayShadow,
    ISampler2D,
    ISampler3D,
    ISamplerCube,
    ISampler2DArray,
    USampler2D,
    USampler3D,
    USamplerCube,
    USampler2DArray,
    Layout,
    Centroid,
    Flat,
    Smooth,
    Switch,
    Case,
    Default,

    /// Reserved for future use (section 3.6): using one is an error.
    Reserved,
}

/// The keyword `s` names, or `None` for an ordinary identifier.
pub fn keyword(s: &str) -> Option<Kw> {
    Some(match s {
        "void" => Kw::Void,
        "bool" => Kw::Bool,
        "int" => Kw::Int,
        "float" => Kw::Float,
        "vec2" => Kw::Vec2,
        "vec3" => Kw::Vec3,
        "vec4" => Kw::Vec4,
        "bvec2" => Kw::BVec2,
        "bvec3" => Kw::BVec3,
        "bvec4" => Kw::BVec4,
        "ivec2" => Kw::IVec2,
        "ivec3" => Kw::IVec3,
        "ivec4" => Kw::IVec4,
        "mat2" => Kw::Mat2,
        "mat3" => Kw::Mat3,
        "mat4" => Kw::Mat4,
        "sampler2D" => Kw::Sampler2D,
        "samplerCube" => Kw::SamplerCube,
        "struct" => Kw::Struct,

        "attribute" => Kw::Attribute,
        "const" => Kw::Const,
        "uniform" => Kw::Uniform,
        "varying" => Kw::Varying,
        "in" => Kw::In,
        "out" => Kw::Out,
        "inout" => Kw::InOut,
        "invariant" => Kw::Invariant,
        "precision" => Kw::Precision,
        "highp" => Kw::HighP,
        "mediump" => Kw::MediumP,
        "lowp" => Kw::LowP,

        "break" => Kw::Break,
        "continue" => Kw::Continue,
        "do" => Kw::Do,
        "for" => Kw::For,
        "while" => Kw::While,
        "if" => Kw::If,
        "else" => Kw::Else,
        "return" => Kw::Return,
        "discard" => Kw::Discard,

        "true" => Kw::True,
        "false" => Kw::False,

        /* Section 3.6, "keywords reserved for future use". Every one of
         * these is an error wherever it appears, including as a variable
         * name, which is why they are classified here rather than left as
         * identifiers for a later pass to miss. */
        "asm"
        | "class"
        | "union"
        | "enum"
        | "typedef"
        | "template"
        | "this"
        | "packed"
        | "goto"
        | "switch"
        | "default"
        | "inline"
        | "noinline"
        | "volatile"
        | "public"
        | "static"
        | "extern"
        | "external"
        | "interface"
        | "flat"
        | "long"
        | "short"
        | "double"
        | "half"
        | "fixed"
        | "unsigned"
        | "superp"
        | "input"
        | "output"
        | "hvec2"
        | "hvec3"
        | "hvec4"
        | "dvec2"
        | "dvec3"
        | "dvec4"
        | "fvec2"
        | "fvec3"
        | "fvec4"
        | "sampler1D"
        | "sampler3D"
        | "sampler1DShadow"
        | "sampler2DShadow"
        | "sampler2DRect"
        | "sampler3DRect"
        | "sampler2DRectShadow"
        | "sizeof"
        | "cast"
        | "namespace"
        | "using" => Kw::Reserved,

        _ => return None,
    })
}

/// The keyword `s` names in GLSL ES 3.00 (section 3.8 of that
/// specification), or `None` for an ordinary identifier.
///
/// Not a superset of [`keyword`]: `attribute` and `varying` are RESERVED in
/// 3.00, and 1.00's reserved `switch`, `default`, `flat`, `sampler3D` and
/// `sampler2DShadow` are keywords. The lists are dEQP-GLES3's
/// `keywords.keywords` and `keywords.reserved_keywords`, which transcribe the
/// specification's; its `allowed_keywords` (`image1DShadow` and the like)
/// are ordinary identifiers.
pub fn keyword_es3(s: &str) -> Option<Kw> {
    Some(match s {
        "uint" => Kw::Uint,
        "uvec2" => Kw::UVec2,
        "uvec3" => Kw::UVec3,
        "uvec4" => Kw::UVec4,
        "mat2x2" => Kw::Mat2,
        "mat3x3" => Kw::Mat3,
        "mat4x4" => Kw::Mat4,
        "mat2x3" => Kw::MatCxR(2, 3),
        "mat2x4" => Kw::MatCxR(2, 4),
        "mat3x2" => Kw::MatCxR(3, 2),
        "mat3x4" => Kw::MatCxR(3, 4),
        "mat4x2" => Kw::MatCxR(4, 2),
        "mat4x3" => Kw::MatCxR(4, 3),
        "sampler3D" => Kw::Sampler3D,
        "sampler2DShadow" => Kw::Sampler2DShadow,
        "samplerCubeShadow" => Kw::SamplerCubeShadow,
        "sampler2DArray" => Kw::Sampler2DArray,
        "sampler2DArrayShadow" => Kw::Sampler2DArrayShadow,
        "isampler2D" => Kw::ISampler2D,
        "isampler3D" => Kw::ISampler3D,
        "isamplerCube" => Kw::ISamplerCube,
        "isampler2DArray" => Kw::ISampler2DArray,
        "usampler2D" => Kw::USampler2D,
        "usampler3D" => Kw::USampler3D,
        "usamplerCube" => Kw::USamplerCube,
        "usampler2DArray" => Kw::USampler2DArray,
        "layout" => Kw::Layout,
        "centroid" => Kw::Centroid,
        "flat" => Kw::Flat,
        "smooth" => Kw::Smooth,
        "switch" => Kw::Switch,
        "case" => Kw::Case,
        "default" => Kw::Default,

        "attribute"
        | "varying"
        | "coherent"
        | "volatile"
        | "restrict"
        | "readonly"
        | "writeonly"
        | "resource"
        | "atomic_uint"
        | "noperspective"
        | "patch"
        | "sample"
        | "subroutine"
        | "common"
        | "partition"
        | "active"
        | "asm"
        | "class"
        | "union"
        | "enum"
        | "typedef"
        | "template"
        | "this"
        | "goto"
        | "inline"
        | "noinline"
        | "public"
        | "static"
        | "extern"
        | "external"
        | "interface"
        | "long"
        | "short"
        | "double"
        | "half"
        | "fixed"
        | "unsigned"
        | "superp"
        | "input"
        | "output"
        | "hvec2"
        | "hvec3"
        | "hvec4"
        | "dvec2"
        | "dvec3"
        | "dvec4"
        | "fvec2"
        | "fvec3"
        | "fvec4"
        | "sampler3DRect"
        | "filter"
        | "image1D"
        | "image2D"
        | "image3D"
        | "imageCube"
        | "iimage1D"
        | "iimage2D"
        | "iimage3D"
        | "iimageCube"
        | "uimage1D"
        | "uimage2D"
        | "uimage3D"
        | "uimageCube"
        | "image1DArray"
        | "image2DArray"
        | "iimage1DArray"
        | "iimage2DArray"
        | "uimage1DArray"
        | "uimage2DArray"
        | "imageBuffer"
        | "iimageBuffer"
        | "uimageBuffer"
        | "sampler1D"
        | "sampler1DShadow"
        | "sampler1DArray"
        | "sampler1DArrayShadow"
        | "isampler1D"
        | "isampler1DArray"
        | "usampler1D"
        | "usampler1DArray"
        | "sampler2DRect"
        | "sampler2DRectShadow"
        | "isampler2DRect"
        | "usampler2DRect"
        | "samplerBuffer"
        | "isamplerBuffer"
        | "usamplerBuffer"
        | "sampler2DMS"
        | "isampler2DMS"
        | "usampler2DMS"
        | "sampler2DMSArray"
        | "isampler2DMSArray"
        | "usampler2DMSArray"
        | "sizeof"
        | "cast"
        | "namespace"
        | "using" => Kw::Reserved,

        /* What 1.00 reserved and 3.00 does not ("packed") is an identifier
         * here; the rest of 1.00's keywords are 3.00's too. */
        "packed" => return None,
        _ => return keyword(s),
    })
}

/// Which language version's lexical rules to apply. Only the rules that
/// differ are switched here; everything else is shared.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Version {
    /// `#version 100`, or no `#version` at all (section 3.3).
    Es100,
    /// `#version 300 es` and later: backslash-newline continues a line, and
    /// a float constant may carry an `f` suffix.
    Es300,
}

impl Version {
    fn line_continuation(self) -> bool {
        self == Version::Es300
    }

    fn float_suffix(self) -> bool {
        self == Version::Es300
    }
}

/// Split `src` into tokens, ending with one [`Tok::Eof`].
pub fn tokenize<'a>(a: &Arena<'a>, src: &'a str, version: Version) -> R<'a, &'a [Token<'a>]> {
    let src = if version.line_continuation() { splice_lines(a, src)? } else { src };
    let mut lx = Lexer::new(src, version);
    /* One token per four source bytes is a close over-estimate for GLSL and
     * saves most of the vector's regrowth copies. */
    let mut out = AVec::with_capacity(a, src.len() / 4 + 8)?;
    let es3 = version == Version::Es300;
    loop {
        let mut t = lx.next_token()?;
        t.es3 = es3;
        /* ES 3.00 section 3.8: "The maximum length of an identifier is 1024
         * characters" -- a longer one is an error (dEQP-GLES3
         * keywords.invalid_identifiers.max_length). ES 1.00 sets no limit. */
        if es3 && t.kind == Tok::Ident && t.text.len() > 1024 {
            return Err(Error::new(t.line, &t.text[..32], "an identifier is longer than 1024 characters"));
        }
        let end = t.kind == Tok::Eof;
        out.push(a, t)?;
        if end {
            return Ok(out.into_slice());
        }
    }
}

/// ES 3.00 section 3.1: a backslash immediately before a line terminator
/// joins the two lines BEFORE anything else happens, so the join may fall
/// inside a token or a directive name (`#def\` / `ine`, `f\` / `oo`: dEQP's
/// `line_continuation.preprocessing_token` and `.token`). The source is
/// copied with each backslash-newline removed, and the removed line ends are
/// put back at the end of the logical line, so every later line keeps its
/// number. A source with no continuation is returned as it is.
fn splice_lines<'a>(a: &Arena<'a>, src: &'a str) -> R<'a, &'a str> {
    let b = src.as_bytes();
    let joins = |k: usize| -> usize {
        match (b.get(k + 1), b.get(k + 2)) {
            (Some(b'\r'), Some(b'\n')) => 3,
            (Some(b'\n'), _) | (Some(b'\r'), _) => 2,
            _ => 0,
        }
    };
    if !(0..b.len()).any(|k| b[k] == b'\\' && joins(k) != 0) {
        return Ok(src);
    }
    let mut out: AVec<'a, u8> = AVec::with_capacity(a, b.len())?;
    let (mut k, mut owed) = (0usize, 0usize);
    while k < b.len() {
        if b[k] == b'\\' {
            let n = joins(k);
            if n != 0 {
                k += n;
                owed += 1;
                continue;
            }
        }
        out.push(a, b[k])?;
        if b[k] == b'\n' || (b[k] == b'\r' && b.get(k + 1) != Some(&b'\n')) {
            for _ in 0..owed {
                out.push(a, b'\n')?;
            }
            owed = 0;
        }
        k += 1;
    }
    for _ in 0..owed {
        out.push(a, b'\n')?;
    }
    /* Only ASCII bytes were removed, so what is left is still UTF-8. */
    core::str::from_utf8(out.into_slice()).map_err(|_| Error::new(1, "\\", "the source is not UTF-8"))
}

/// The token scanner. [`tokenize`] is the usual entry point; the scanner is
/// public so a test can step it one token at a time.
pub struct Lexer<'a> {
    src: &'a str,
    b: &'a [u8],
    pos: usize,
    line: u32,
    bol: bool,
    version: Version,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, version: Version) -> Lexer<'a> {
        Lexer { src, b: src.as_bytes(), pos: 0, line: 1, bol: true, version }
    }

    fn at(&self, off: usize) -> u8 {
        *self.b.get(self.pos + off).unwrap_or(&0)
    }

    /// Whitespace and comments. Returns true if any was skipped, and leaves
    /// `self.bol` set when a line ended on the way.
    fn skip_trivia(&mut self) -> R<'a, bool> {
        let start = self.pos;
        loop {
            match self.at(0) {
                b'\n' => {
                    self.pos += 1;
                    self.line += 1;
                    self.bol = true;
                }
                b'\r' => {
                    /* A CR, or a CRLF pair, is one line terminator
                     * (section 3.1). Counting CRLF as two would put every
                     * later diagnostic on the wrong line of a DOS-ended
                     * file. */
                    self.pos += 1;
                    if self.at(0) == b'\n' {
                        self.pos += 1;
                    }
                    self.line += 1;
                    self.bol = true;
                }
                b' ' | b'\t' | 0x0b | 0x0c => self.pos += 1,
                b'\\' if self.version.line_continuation() => {
                    /* Backslash-newline (ES 3.00 only) splices the lines:
                     * the physical line ends but the logical one does not,
                     * so `bol` is deliberately left alone. */
                    let mut n = 1;
                    if self.at(n) == b'\r' {
                        n += 1;
                    }
                    if self.at(n) == b'\n' {
                        self.pos += n + 1;
                        self.line += 1;
                    } else {
                        /* A backslash that does not continue a line is not a
                         * GLSL character; leave it to `unknown`. */
                        break;
                    }
                }
                b'/' if self.at(1) == b'/' => {
                    while self.pos < self.b.len() && self.at(0) != b'\n' && self.at(0) != b'\r' {
                        /* ES 3.00 splices lines BEFORE comments are
                         * recognised (section 3.1 of that specification), so
                         * a `//` comment ending in a backslash goes on to
                         * the next line: dEQP-GLES3
                         * preprocessor.line_continuation.comment. */
                        if self.at(0) == b'\\' && self.version.line_continuation() {
                            let eol = match (self.at(1), self.at(2)) {
                                (b'\r', b'\n') => 3,
                                (b'\n', _) | (b'\r', _) => 2,
                                _ => 0,
                            };
                            if eol != 0 {
                                self.pos += eol;
                                self.line += 1;
                                continue;
                            }
                        }
                        self.pos += 1;
                    }
                }
                b'/' if self.at(1) == b'*' => {
                    let open = self.line;
                    self.pos += 2;
                    loop {
                        if self.pos >= self.b.len() {
                            return Err(Error::new(open, "/*", "end of file in comment"));
                        }
                        if self.at(0) == b'*' && self.at(1) == b'/' {
                            self.pos += 2;
                            break;
                        }
                        if self.at(0) == b'\n' {
                            /* The line COUNT advances, so diagnostics stay on
                             * the application's own lines, but `bol` does
                             * not: a comment is one space (C's translation
                             * phase 3), so a directive survives a comment
                             * that spans lines. dEQP's
                             * function_redefinition_3 puts one in the middle
                             * of a `#define` body and expects the shader to
                             * build; ending the directive there truncates the
                             * replacement list and the next, identical
                             * definition then looks like a redefinition. */
                            self.line += 1;
                        }
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
        Ok(self.pos != start)
    }

    /// The next token, or [`Tok::Eof`] at the end of the input.
    ///
    /// Deliberately not `Iterator::next`: scanning can fail, and an iterator
    /// that reported a lexical error as `None` would look exactly like the
    /// end of the file.
    pub fn next_token(&mut self) -> R<'a, Token<'a>> {
        let ws = self.skip_trivia()?;
        let bol = self.bol;
        self.bol = false;
        let line = self.line;
        if self.pos >= self.b.len() {
            return Ok(Token::new(Tok::Eof, "", line, bol, ws));
        }
        let c = self.at(0);
        if c.is_ascii_alphabetic() || c == b'_' {
            return Ok(self.ident(line, bol, ws));
        }
        if c.is_ascii_digit() || (c == b'.' && self.at(1).is_ascii_digit()) {
            return self.number(line, bol, ws);
        }
        if !c.is_ascii() {
            /* Section 3.1: the source character set outside comments is
             * ASCII. The run is taken whole so that the token's text stays on
             * a character boundary and `self.src` slicing cannot panic. */
            return Ok(self.unknown(line, bol, ws));
        }
        self.punct(line, bol, ws)
    }

    /// A run of characters that is in no GLSL token.
    fn unknown(&mut self, line: u32, bol: bool, ws: bool) -> Token<'a> {
        let start = self.pos;
        loop {
            self.pos += 1;
            if self.pos >= self.b.len() || self.src.is_char_boundary(self.pos) {
                break;
            }
        }
        Token::new(Tok::Unknown, &self.src[start..self.pos], line, bol, ws)
    }

    fn ident(&mut self, line: u32, bol: bool, ws: bool) -> Token<'a> {
        let start = self.pos;
        while self.at(0).is_ascii_alphanumeric() || self.at(0) == b'_' {
            self.pos += 1;
        }
        Token::new(Tok::Ident, &self.src[start..self.pos], line, bol, ws)
    }

    /// A numeric constant (sections 4.1.3 and 4.1.4).
    fn number(&mut self, line: u32, bol: bool, ws: bool) -> R<'a, Token<'a>> {
        let start = self.pos;
        if self.at(0) == b'0' && (self.at(1) | 0x20) == b'x' {
            self.pos += 2;
            let digits = self.pos;
            while self.at(0).is_ascii_hexdigit() {
                self.pos += 1;
            }
            if self.pos == digits {
                return Err(Error::new(line, &self.src[start..self.pos], "no digits after the '0x' prefix"));
            }
            let mut v: u32 = 0;
            for &d in &self.b[digits..self.pos] {
                let n = (d as char).to_digit(16).unwrap_or(0);
                v = v.wrapping_mul(16).wrapping_add(n);
            }
            return self.finish_int(v, start, line, bol, ws);
        }

        let mut is_float = false;
        while self.at(0).is_ascii_digit() {
            self.pos += 1;
        }
        if self.at(0) == b'.' {
            is_float = true;
            self.pos += 1;
            while self.at(0).is_ascii_digit() {
                self.pos += 1;
            }
        }
        if (self.at(0) | 0x20) == b'e' {
            /* An `e` only starts an exponent when digits follow it, so that
             * `1east` stays "1" and an identifier rather than becoming a
             * malformed float. */
            let mut n = 1;
            if self.at(n) == b'+' || self.at(n) == b'-' {
                n += 1;
            }
            if self.at(n).is_ascii_digit() {
                is_float = true;
                self.pos += n;
                while self.at(0).is_ascii_digit() {
                    self.pos += 1;
                }
            }
        }

        if !is_float {
            let text = &self.src[start..self.pos];
            let v = if text.len() > 1 && text.as_bytes()[0] == b'0' {
                /* A leading zero means octal (section 4.1.3). `08` has no
                 * octal digit 8; saying so beats silently reading it as
                 * decimal, which is what a permissive parser would do. */
                let mut v: u32 = 0;
                for &d in text.as_bytes() {
                    if !d.is_ascii_digit() || d > b'7' {
                        return Err(Error::new(line, text, "octal constant has a digit above 7"));
                    }
                    v = v.wrapping_mul(8).wrapping_add((d - b'0') as u32);
                }
                v
            } else {
                let mut v: u32 = 0;
                for &d in text.as_bytes() {
                    v = v.wrapping_mul(10).wrapping_add((d - b'0') as u32);
                }
                v
            };
            return self.finish_int(v, start, line, bol, ws);
        }

        let digits_end = self.pos;
        if self.version.float_suffix() && (self.at(0) | 0x20) == b'f' {
            self.pos += 1;
        }
        if self.at(0).is_ascii_alphanumeric() || self.at(0) == b'_' {
            /* `1.0f` in ES 1.00, or `1.0x` in any version: a constant glued
             * to an identifier. Section 3.7 has no such token, and letting
             * it through would quietly split it into two. */
            let end = self.pos + 1;
            return Err(Error::new(line, &self.src[start..end], "invalid suffix on a floating-point constant"));
        }
        let text = &self.src[start..self.pos];
        let Some(value) = parse_f32(&self.src[start..digits_end]) else {
            return Err(Error::new(line, text, "cannot represent this floating-point constant"));
        };
        let mut t = Token::new(Tok::FloatLit, text, line, bol, ws);
        t.float = value;
        Ok(t)
    }

    /// Finish an integer constant, after rejecting anything glued to its
    /// tail.
    ///
    /// C and GLSL both scan a number as one "preprocessing number" and then
    /// check it, so `1east` is one bad token rather than `1` followed by the
    /// identifier `east`. Splitting it silently is the failure mode this
    /// guards: the shader would compile with a meaning its author never
    /// wrote. ES 3.00's `u`/`U` suffix is the one legal tail.
    fn finish_int(&mut self, v: u32, start: usize, line: u32, bol: bool, ws: bool) -> R<'a, Token<'a>> {
        if self.version == Version::Es300 && (self.at(0) | 0x20) == b'u' {
            self.pos += 1;
        }
        if self.at(0).is_ascii_alphanumeric() || self.at(0) == b'_' {
            let end = self.pos + 1;
            return Err(Error::new(line, &self.src[start..end], "invalid suffix on an integer constant"));
        }
        let mut t = Token::new(Tok::IntLit, &self.src[start..self.pos], line, bol, ws);
        t.int = v;
        Ok(t)
    }

    fn punct(&mut self, line: u32, bol: bool, ws: bool) -> R<'a, Token<'a>> {
        let start = self.pos;
        let pair = [self.at(0), self.at(1)];
        let kind = if (pair == *b"<<" || pair == *b">>") && self.at(2) == b'=' {
            self.pos += 3;
            if pair == *b"<<" {
                Tok::ShlAssign
            } else {
                Tok::ShrAssign
            }
        } else {
            let two_char = match &pair {
                b"++" => Some(Tok::Inc),
                b"--" => Some(Tok::Dec),
                b"<=" => Some(Tok::Le),
                b">=" => Some(Tok::Ge),
                b"==" => Some(Tok::EqEq),
                b"!=" => Some(Tok::NotEq),
                b"&&" => Some(Tok::AndAnd),
                b"||" => Some(Tok::OrOr),
                b"^^" => Some(Tok::XorXor),
                b"<<" => Some(Tok::Shl),
                b">>" => Some(Tok::Shr),
                b"+=" => Some(Tok::AddAssign),
                b"-=" => Some(Tok::SubAssign),
                b"*=" => Some(Tok::MulAssign),
                b"/=" => Some(Tok::DivAssign),
                b"%=" => Some(Tok::ModAssign),
                b"&=" => Some(Tok::AndAssign),
                b"|=" => Some(Tok::OrAssign),
                b"^=" => Some(Tok::XorAssign),
                b"##" => Some(Tok::HashHash),
                _ => None,
            };
            match two_char {
                Some(k) => {
                    self.pos += 2;
                    k
                }
                None => {
                    let one = match self.at(0) {
                        b'(' => Tok::LParen,
                        b')' => Tok::RParen,
                        b'{' => Tok::LBrace,
                        b'}' => Tok::RBrace,
                        b'[' => Tok::LBracket,
                        b']' => Tok::RBracket,
                        b'.' => Tok::Dot,
                        b',' => Tok::Comma,
                        b':' => Tok::Colon,
                        b';' => Tok::Semi,
                        b'?' => Tok::Question,
                        b'+' => Tok::Plus,
                        b'-' => Tok::Minus,
                        b'*' => Tok::Star,
                        b'/' => Tok::Slash,
                        b'%' => Tok::Percent,
                        b'<' => Tok::Lt,
                        b'>' => Tok::Gt,
                        b'!' => Tok::Not,
                        b'~' => Tok::Tilde,
                        b'&' => Tok::Amp,
                        b'|' => Tok::Pipe,
                        b'^' => Tok::Caret,
                        b'=' => Tok::Assign,
                        b'#' => Tok::Hash,
                        /* Not a GLSL character: a token, so that a `#pragma`
                         * may hold it (see [`Tok::Unknown`]). */
                        _ => return Ok(self.unknown(line, bol, ws)),
                    };
                    self.pos += 1;
                    one
                }
            }
        };
        Ok(Token::new(kind, &self.src[start..self.pos], line, bol, ws))
    }
}

/// A floating-point constant's value.
///
/// `core`'s own parser does the conversion: it is correctly rounded, and
/// writing a second one is how a compiler acquires a rounding bug. GLSL's
/// two odd spellings, `1.` and `.5`, are inside Rust's grammar as well
/// (`Significand ::= Digits '.' [Digits] | [Digits] '.' Digits | Digits`),
/// which `float_constants_cover_every_glsl_spelling` pins so that a future
/// `core` cannot quietly stop accepting them.
fn parse_f32(text: &str) -> Option<f32> {
    text.parse::<f32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tokenize `src` in a fresh arena over `buf`.
    fn lex<'a>(buf: &'a mut [u8], src: &'a str, v: Version) -> R<'a, &'a [Token<'a>]> {
        let a = Arena::new(buf);
        tokenize(&a, src, v)
    }

    fn kinds(t: &[Token]) -> std::vec::Vec<Tok> {
        t.iter().map(|t| t.kind).collect()
    }

    #[test]
    fn a_minimal_shader_lexes() {
        let mut buf = [0u8; 8192];
        let t = lex(&mut buf, "void main() { gl_FragColor = vec4(1.0); }", Version::Es100).expect("lex");
        assert_eq!(
            kinds(t),
            [
                Tok::Ident,
                Tok::Ident,
                Tok::LParen,
                Tok::RParen,
                Tok::LBrace,
                Tok::Ident,
                Tok::Assign,
                Tok::Ident,
                Tok::LParen,
                Tok::FloatLit,
                Tok::RParen,
                Tok::Semi,
                Tok::RBrace,
                Tok::Eof,
            ]
        );
        assert_eq!(t[0].kw(), Some(Kw::Void));
        assert_eq!(t[1].kw(), None, "main is not a keyword");
    }

    #[test]
    fn line_numbers_survive_comments_and_crlf() {
        let mut buf = [0u8; 8192];
        let src = "a\r\nb\n/* two\n   lines */ c\n// end\nd";
        let t = lex(&mut buf, src, Version::Es100).expect("lex");
        let lines: std::vec::Vec<u32> = t.iter().filter(|t| t.kind == Tok::Ident).map(|t| t.line).collect();
        assert_eq!(lines, [1, 2, 4, 6], "CRLF is one line and a block comment advances the count");
    }

    #[test]
    fn bol_marks_the_first_token_of_a_line_and_a_comment_does_not_clear_it() {
        let mut buf = [0u8; 8192];
        let t = lex(&mut buf, "a b\n  /* x */ #define C 1", Version::Es100).expect("lex");
        assert!(t[0].bol, "first token of the file");
        assert!(!t[1].bol, "second token of line 1");
        assert!(t[2].bol, "the '#' is first on line 2 even behind whitespace and a comment");
        assert_eq!(t[2].kind, Tok::Hash);
    }

    #[test]
    fn ws_before_separates_a_function_like_macro_from_an_object_like_one() {
        let mut buf = [0u8; 8192];
        let f = lex(&mut buf, "A(x)", Version::Es100).expect("lex");
        assert!(!f[1].ws_before, "A(x): the paren touches the name");
        let mut buf2 = [0u8; 8192];
        let o = lex(&mut buf2, "A (x)", Version::Es100).expect("lex");
        assert!(o[1].ws_before, "A (x): the paren is separated");
    }

    #[test]
    fn integer_constants_take_their_base_from_the_prefix() {
        let mut buf = [0u8; 8192];
        let t = lex(&mut buf, "0 7 10 017 0x1F 0XfF 4294967295", Version::Es100).expect("lex");
        let v: std::vec::Vec<u32> = t.iter().filter(|t| t.kind == Tok::IntLit).map(|t| t.int).collect();
        assert_eq!(v, [0, 7, 10, 15, 31, 255, 4294967295]);
    }

    #[test]
    fn an_octal_digit_above_seven_is_an_error() {
        let mut buf = [0u8; 8192];
        let e = lex(&mut buf, "int i = 08;", Version::Es100).expect_err("08 is not octal");
        assert_eq!(e.tok, "08");
        assert_eq!(e.msg, "octal constant has a digit above 7");
    }

    #[test]
    fn float_constants_cover_every_glsl_spelling() {
        let mut buf = [0u8; 8192];
        let t = lex(&mut buf, "1.0 1. .5 0.5 1e3 1.5e-3 2E+2 .25e1 0.1", Version::Es100).expect("lex");
        let v: std::vec::Vec<f32> = t.iter().filter(|t| t.kind == Tok::FloatLit).map(|t| t.float).collect();
        assert_eq!(v.len(), 9, "every one of them is exactly one float token");
        assert_eq!(v, [1.0f32, 1.0, 0.5, 0.5, 1000.0, 1.5e-3, 200.0, 2.5, 0.1], "and correctly rounded");
    }

    /// ES 1.00 has no `f` suffix; it arrived in ES 3.00. A front end that
    /// accepted it would compile shaders the specification says must fail.
    #[test]
    fn the_f_suffix_is_an_error_in_es100_and_a_constant_in_es300() {
        let mut buf = [0u8; 8192];
        let e = lex(&mut buf, "float x = 1.0f;", Version::Es100).expect_err("no f suffix in ES 1.00");
        assert_eq!(e.msg, "invalid suffix on a floating-point constant");
        assert_eq!(e.tok, "1.0f");

        let mut buf2 = [0u8; 8192];
        let t = lex(&mut buf2, "float x = 1.0f;", Version::Es300).expect("ES 3.00 allows it");
        let f: std::vec::Vec<f32> = t.iter().filter(|t| t.kind == Tok::FloatLit).map(|t| t.float).collect();
        assert_eq!(f, [1.0]);
    }

    /// Likewise line continuation: ES 1.00 section 3.1 says there is none, so
    /// the backslash is simply not a GLSL character there.
    #[test]
    fn backslash_is_an_error_in_es100_and_a_continuation_in_es300() {
        let mut buf = [0u8; 8192];
        let t = lex(&mut buf, "int a\\\n= 1;", Version::Es100).expect("lex");
        assert_eq!(kinds(t)[2], Tok::Unknown, "not a continuation, and not a token either");
        assert!(t[3].bol, "the next line really is a new line");

        let mut buf2 = [0u8; 8192];
        let t = lex(&mut buf2, "int a\\\n= 1;", Version::Es300).expect("ES 3.00 splices the line");
        assert_eq!(kinds(t), [Tok::Ident, Tok::Ident, Tok::Assign, Tok::IntLit, Tok::Semi, Tok::Eof]);
        assert!(!t[2].bol, "a spliced line is one logical line");
    }

    /// The join happens before tokens are formed, so it may split one
    /// (dEQP-GLES3 `preprocessor.line_continuation.token`), and the lines
    /// after it keep their numbers.
    ///
    /// Revert check: tokenize the source unspliced and `f\` / `oo` is two
    /// identifiers and a stray backslash.
    #[test]
    fn a_continuation_may_fall_inside_a_token_and_later_lines_keep_their_numbers() {
        let mut buf = [0u8; 8192];
        let t = lex(&mut buf, "float f\\\noo = 1.0;\r\nint z;", Version::Es300).expect("lex");
        assert_eq!(kinds(t)[..3], [Tok::Ident, Tok::Ident, Tok::Assign]);
        assert_eq!(t[1].text, "foo");
        let z = t.iter().find(|t| t.text == "z").expect("z");
        assert_eq!(z.line, 3, "the third physical line");
    }

    #[test]
    fn operators_lex_longest_first() {
        let mut buf = [0u8; 8192];
        let t = lex(&mut buf, "<<= >>= << >> <= >= == != && || ^^ ++ -- += -= *= /= %= &= |= ^= ## #", Version::Es100)
            .expect("lex");
        assert_eq!(
            kinds(t),
            [
                Tok::ShlAssign,
                Tok::ShrAssign,
                Tok::Shl,
                Tok::Shr,
                Tok::Le,
                Tok::Ge,
                Tok::EqEq,
                Tok::NotEq,
                Tok::AndAnd,
                Tok::OrOr,
                Tok::XorXor,
                Tok::Inc,
                Tok::Dec,
                Tok::AddAssign,
                Tok::SubAssign,
                Tok::MulAssign,
                Tok::DivAssign,
                Tok::ModAssign,
                Tok::AndAssign,
                Tok::OrAssign,
                Tok::XorAssign,
                Tok::HashHash,
                Tok::Hash,
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn reserved_words_are_classified_rather_than_passed_through() {
        assert_eq!(keyword("switch"), Some(Kw::Reserved));
        assert_eq!(keyword("half"), Some(Kw::Reserved));
        assert_eq!(keyword("sampler3D"), Some(Kw::Reserved), "ES 1.00 has no sampler3D");
        assert_eq!(keyword("texture2D"), None, "a built-in function is an ordinary identifier");
    }

    #[test]
    fn an_unterminated_comment_reports_where_it_opened() {
        let mut buf = [0u8; 8192];
        let e = lex(&mut buf, "int a;\n/* never closed\nmore\n", Version::Es100).expect_err("unterminated");
        assert_eq!(e.line, 2, "the line the comment opened on, not the end of the file");
        assert_eq!(e.msg, "end of file in comment");
    }

    /// A character outside the source character set is an `Unknown` token,
    /// not a lexical error: `#pragma` may hold anything (dEQP
    /// pragmas.pragma_unrecognized_token expects that shader to BUILD), so
    /// the preprocessor is what rejects one that reaches live code.
    #[test]
    fn a_character_outside_the_source_set_becomes_one_unknown_token() {
        let mut buf = [0u8; 8192];
        let t = lex(&mut buf, "int \u{00e9} = 1;", Version::Es100).expect("lex");
        assert_eq!(kinds(t), [Tok::Ident, Tok::Unknown, Tok::Assign, Tok::IntLit, Tok::Semi, Tok::Eof]);
        assert_eq!(t[1].text, "\u{00e9}", "the whole character, so the text stays on a boundary");

        let mut buf2 = [0u8; 8192];
        let t = lex(&mut buf2, "int @ $ = 1;", Version::Es100).expect("lex");
        assert_eq!(kinds(t), [Tok::Ident, Tok::Unknown, Tok::Unknown, Tok::Assign, Tok::IntLit, Tok::Semi, Tok::Eof]);

        let mut buf3 = [0u8; 8192];
        let t = lex(&mut buf3, "/* caf\u{00e9} */ int a;", Version::Es100).expect("comments may hold anything");
        assert_eq!(kinds(t), [Tok::Ident, Tok::Ident, Tok::Semi, Tok::Eof]);
    }

    /// A comment is one space (C's translation phase 3), so a comment that
    /// spans lines advances the line COUNT but does not start a new logical
    /// line. dEQP's function_redefinition_3 puts one inside a `#define` body
    /// and expects the shader to build: treating it as a line break truncates
    /// the replacement list, and the next identical `#define` then looks like
    /// a redefinition with different text.
    #[test]
    fn a_comment_that_spans_lines_does_not_start_a_new_logical_line() {
        let mut buf = [0u8; 8192];
        let t = lex(&mut buf, "a /* one\n   two */ b\nc", Version::Es100).expect("lex");
        assert!(t[0].bol, "a starts line 1");
        assert!(!t[1].bol, "b is still on the same logical line as a");
        assert_eq!(t[1].line, 2, "but the line count followed the comment");
        assert!(t[2].bol, "c is a new line");
        assert_eq!(t[2].line, 3);
    }

    #[test]
    fn eof_is_always_the_last_token_so_lookahead_needs_no_bounds_check() {
        for src in ["", "   ", "// only a comment", "a", "/* c */"] {
            let mut buf = [0u8; 8192];
            let t = lex(&mut buf, src, Version::Es100).expect("lex");
            assert_eq!(t.last().map(|t| t.kind), Some(Tok::Eof), "{src:?}");
        }
    }

    /// C and GLSL scan a number as one preprocessing-number token, so a
    /// letter glued to a constant is one bad token. Splitting `1east` into
    /// `1` and `east` would compile a shader whose author wrote neither.
    #[test]
    fn a_letter_glued_to_a_constant_is_one_bad_token() {
        let mut buf = [0u8; 8192];
        let e = lex(&mut buf, "1east", Version::Es100).expect_err("an integer glued to an identifier");
        assert_eq!(e.msg, "invalid suffix on an integer constant", "and `e` without digits is no exponent");

        let mut buf2 = [0u8; 8192];
        let e = lex(&mut buf2, "1u", Version::Es100).expect_err("ES 1.00 has no unsigned literals");
        assert_eq!(e.msg, "invalid suffix on an integer constant");

        let mut buf3 = [0u8; 8192];
        let t = lex(&mut buf3, "1u", Version::Es300).expect("ES 3.00 has them");
        assert_eq!(t[0].kind, Tok::IntLit);
        assert_eq!(t[0].int, 1);
    }
}
