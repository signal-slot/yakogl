// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A disassembler: one decoded GC7000 instruction written the way the
//! description says to write it.
//!
//! The format is not invented here. Each operation carries a display template
//! from the ISA description — `{INSTR_ALU} {DST:align=18}, {SRC0}, {SRC1},
//! void` — and the named fragments it refers to hold the modifier order:
//! `{NAME}{DST_FULL}{SAT}{SKPHP}{UNK}{TYPE}{PMODE}{THREAD}{RMODE}`. So the
//! printer expands the template rather than keeping a second, hand-written
//! account of the same format, and the two cannot drift.
//!
//! Two rendering rules are not decode and would each show up as a disagreement
//! with Mesa's own disassembler:
//!
//! * a **full write mask prints as nothing** — `t1`, not `t1.xyzw`;
//! * an **immediate source is not a register**. Its bits are a 20-bit value,
//!   and printing them as a register and a swizzle gives a plausible operand
//!   that does not exist (`immed0.xwww` where the answer is `1.0:f20`).
//!
//! It writes through [`core::fmt::Write`], so it works against a fixed buffer
//! in a `no_std` build as well as a `String` on the host.

use super::isa::Instr;
use super::isa_tables::{ENUM_DISPLAY, TEMPLATES};
use core::fmt::{Result, Write};

/// The spelling an enumeration gives a value, or "" when it names none.
fn enum_display(name: &str, v: u32) -> &'static str {
    ENUM_DISPLAY
        .iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, vals)| vals.iter().find(|(x, _)| *x == v).map(|(_, s)| *s))
        .unwrap_or("")
}

/// A register group's letter (`t`, `u`, `i`, `th`); "" for an immediate.
fn group_letter(g: u32) -> &'static str {
    enum_display("#reg_group", g)
}

/// One decoded source, as the description prints it.
fn src_text(w: &mut impl Write, instr: &Instr, n: usize) -> Result {
    let Some(s) = instr.src(n) else { return w.write_str("void") };
    if s.is_immediate() {
        // The bits are a value, not a register. Which kind is IMMED_TYPE,
        // which the encoding derives as SRC_AMODE >> 1.
        let payload = s.imm_payload_bits().unwrap_or(0);
        return match (s.amode >> 1) & 3 {
            0 => write_f20(w, payload),
            1 => write!(w, "{}:s20", ((payload as i32) << 12) >> 12),
            2 => write!(w, "{payload}:u20"),
            _ => write!(w, "{payload:#x}:f16"),
        };
    }
    if s.neg {
        w.write_str("-")?;
    }
    if s.abs {
        w.write_str("|")?;
    }
    write!(w, "{}{}", group_letter(s.group as u32), s.reg)?;
    w.write_str(enum_display("#reg_addressing_mode", s.amode as u32))?;
    // The swizzle is always spelled out for a source; only the destination's
    // write mask has an abbreviation.
    w.write_str(".")?;
    for c in s.swizzle {
        w.write_str(enum_display("#swiz", c as u32))?;
    }
    if s.abs {
        w.write_str("|")?;
    }
    Ok(())
}

/// A 20-bit float payload, printed as Mesa prints it.
fn write_f20(w: &mut impl Write, payload: u32) -> Result {
    let v = f32::from_bits(payload << 12);
    if v == libm_trunc(v) && v.abs() < 1.0e7 {
        // 1.0, not 1. Mesa prints one decimal place for a whole number.
        write!(w, "{:.1}:f20", v)
    } else {
        write!(w, "{v}:f20")
    }
}

/// `f32::trunc` is std; this module is `no_std`.
fn libm_trunc(v: f32) -> f32 {
    (v as i64) as f32
}

/// The destination, as the description prints it.
fn dst_text(w: &mut impl Write, instr: &Instr) -> Result {
    // An unused destination prints `void` -- the description says so in an
    // override on #instruction-dst (`{DST_USE} == 0`).
    if instr.get("DST_USE") == Some(0) {
        return w.write_str("void");
    }
    let Some(d) = instr.dst() else { return w.write_str("void") };
    write!(w, "t{}", d.reg)?;
    w.write_str(enum_display("#reg_addressing_mode", d.amode as u32))?;
    // A full write mask prints as nothing: `t1`, not `t1.xyzw`.
    w.write_str(enum_display("#wrmask", d.write_mask as u32))
}

/// Expand `{TOKEN}` against the instruction; returns false when the token is
/// one this printer does not know, so the caller can keep it visible rather
/// than drop it.
fn expand(w: &mut impl Write, instr: &Instr, tok: &str) -> core::result::Result<bool, core::fmt::Error> {
    // `{DST:align=18}` -- the alignment is Mesa's column, applied by the caller.
    let name = tok.split(':').next().unwrap_or(tok);
    // `{:align=18}` with no field name is pure column padding, which
    // `body_text` has already applied to the head of the line. The control-
    // flow forms use it where the ALU forms name a destination.
    if name.is_empty() {
        return Ok(true);
    }
    match name {
        "NAME" => {
            w.write_str(instr.op().mnemonic)?;
            return Ok(true);
        }
        "DST" => {
            dst_text(w, instr)?;
            return Ok(true);
        }
        "SRC0" => {
            src_text(w, instr, 0)?;
            return Ok(true);
        }
        "SRC1" => {
            src_text(w, instr, 1)?;
            return Ok(true);
        }
        "SRC2" => {
            src_text(w, instr, 2)?;
            return Ok(true);
        }
        _ => {}
    }
    // A named fragment: expand it in turn.
    if let Some((_, frag)) = TEMPLATES.iter().find(|(n, _)| *n == name) {
        render(w, instr, frag)?;
        return Ok(true);
    }
    // A value the description assembles from other fields (TYPE, THREAD).
    if let Some((_, parts)) = instr.op().derived.iter().find(|(n, _)| *n == name) {
        let mut v = 0u32;
        for (field, shift) in *parts {
            v |= instr.get(field).unwrap_or(0) << shift;
        }
        // The enumeration it is typed by carries the spelling; the two the
        // templates use are #type and #thread.
        let ty = if name == "TYPE" { "#type" } else { "#thread" };
        w.write_str(enum_display(ty, v))?;
        return Ok(true);
    }

    // A field of this instruction: print the spelling its value carries.
    if let Some((i, f)) = instr.op().fields.iter().enumerate().find(|(_, f)| f.name == name) {
        let v = instr.vals[i];
        if let Some(ty) = f.ty {
            // A packed four-lane swizzle (the texture swizzle is one): print
            // the lanes, not the number. Mesa writes `tex1.xyzw`, and a
            // listing that stops at `tex1` loses which channels the fetch
            // takes -- 103 of the corpus's instructions differ on exactly
            // this and nothing else.
            if ty == "#src-swizzle" {
                w.write_str(".")?;
                for k in 0..4 {
                    w.write_str(enum_display("#swiz", (v >> (2 * k)) & 3))?;
                }
                return Ok(true);
            }
            if ty.starts_with('#') {
                w.write_str(enum_display(ty, v))?;
                return Ok(true);
            }
            if ty == "bool" || ty == "bool_inv" {
                // `bool_inv` is stored inverted: PMODE zero means `.pack` is on.
                let on = if ty == "bool_inv" { v == 0 } else { v != 0 };
                if on {
                    w.write_str(f.display.unwrap_or(""))?;
                }
                return Ok(true);
            }
        }
        write!(w, "{v}")?;
        return Ok(true);
    }
    Ok(false)
}

/// Render a template, writing its literal text and expanding its tokens.
fn render(w: &mut impl Write, instr: &Instr, tmpl: &str) -> Result {
    let mut rest = tmpl;
    while let Some(open) = rest.find('{') {
        w.write_str(&rest[..open])?;
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            return w.write_str(&rest[open..]);
        };
        let tok = &after[..close];
        if !expand(w, instr, tok)? {
            write!(w, "{{{tok}}}")?; // unknown: keep it visible
        }
        rest = &after[close + 1..];
    }
    w.write_str(rest)
}

/// Write one instruction as a listing line, at instruction index `i`, in the
/// shape Mesa's own disassembler uses so the two can be compared line by line.
pub fn instr_text(w: &mut impl Write, i: usize, instr: &Instr) -> Result {
    let dw = super::encode::encode_instr(instr);
    write!(w, "{:03} [{:08x} {:08x} {:08x} {:08x}] ", i, dw[0], dw[1], dw[2], dw[3])?;
    body_text(w, instr)?;
    w.write_str("\n")
}

/// Just the disassembly, without the index and the words.
pub fn body_text(w: &mut impl Write, instr: &Instr) -> Result {
    // The template pads the destination into a column: `{DST:align=18}`. Mesa
    // aligns the head of the line, so build it in two halves.
    let tmpl = instr.op().display;
    match tmpl.find(' ') {
        Some(sp) => {
            let mut head = ColumnBuf::new();
            render(&mut head, instr, &tmpl[..sp])?;
            // Mesa pads the head to 18 columns and the template's own space
            // is part of that padding, not extra: `{:<18}` then the tail
            // WITHOUT its leading space, or every line is one column wide.
            write!(w, "{:<18}", head.as_str())?;
            render(w, instr, &tmpl[sp + 1..])
        }
        None => render(w, instr, tmpl),
    }
}

/// A tiny fixed buffer, so the head of a line can be measured for its column
/// without a heap.
struct ColumnBuf {
    buf: [u8; 64],
    len: usize,
}

impl ColumnBuf {
    fn new() -> ColumnBuf {
        ColumnBuf { buf: [0; 64], len: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
    }
}

impl Write for ColumnBuf {
    fn write_str(&mut self, s: &str) -> Result {
        let b = s.as_bytes();
        let n = b.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&b[..n]);
        self.len += n;
        Ok(())
    }
}
