// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Valhall v10 gates: the board's binaries and Mesa's test vectors
//! decoded, re-encoded byte-equal and described as Mesa describes them; every
//! variant through the encoder; the variant count against the XML; each
//! refusal of the encoder and the decoder by name.
//!
//! The data:
//!
//! * tools/mali/isa-ref/board-2026-10-18.txt: every shader binary of the two
//!   board traces (Mesa 25.0.7 on the NanoPC-T6), each line as pandecode
//!   printed it, made by tools/mali/isa_ref.py.
//! * tools/mali/isa-ref/board-2026-10-18.mesa-26.2.1.txt: the same words
//!   through Mesa 26.2.1's own disassembler (tools/mali/va_disasm_ref.sh; the
//!   instrument reproduces assembler-cases.txt line for line).
//! * Mesa 26.2.1's valhall/test/assembler-cases.txt (vendored), which Mesa's
//!   own valhall_disasm test checks its disassembler against.
//!
//! Every count is pinned, so a fixture that grows or shrinks fails the pin
//! instead of quietly changing what the gate covers.

use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

static BOARD_25: &str = include_str!("../../../../../tools/mali/isa-ref/board-2026-10-18.txt");
static BOARD_26: &str = include_str!("../../../../../tools/mali/isa-ref/board-2026-10-18.mesa-26.2.1.txt");
static VECTORS: &str = include_str!(
    "../../../../../third_party/mesa-26.2.1/src/panfrost/compiler/bifrost/valhall/test/assembler-cases.txt"
);
static KRAID_XML: &str =
    include_str!("../../../../../third_party/mesa-26.2.1/src/panfrost/compiler/kraid/isa-v9-v14.xml");

/// The pins.
const BOARD_BINARIES: usize = 11;
const BOARD_DISTINCT_BINARIES: usize = 7;
const BOARD_INSTRUCTIONS: usize = 132;
const VECTOR_LINES: usize = 246;
const V10_VARIANTS: usize = 365;
const V10_ALIASES: usize = 60;

/// "xx xx xx xx xx xx xx xx    text": the word (bytes in memory order) and
/// the text. Anything else that is not a comment or blank fails.
fn parse_line(l: &str) -> (u64, &str) {
    assert!(l.len() > 27 && &l[23..27] == "    ", "not a word line: {l:?}");
    let mut w = 0u64;
    for (i, b) in l[..23].split(' ').enumerate() {
        assert!(i < 8, "not a word line: {l:?}");
        w |= u64::from(u8::from_str_radix(b, 16).unwrap_or_else(|e| panic!("{l:?}: {e}"))) << (8 * i);
    }
    (w, &l[27..])
}

/// The binaries of a board fixture: (`# binary` label, its lines).
fn binaries(text: &str) -> Vec<(String, Vec<(u64, String)>)> {
    let mut out: Vec<(String, Vec<(u64, String)>)> = Vec::new();
    for l in text.lines() {
        if let Some(b) = l.strip_prefix("# binary ") {
            out.push((b.to_string(), Vec::new()));
        } else if l.starts_with('#') {
            assert!(out.is_empty(), "a comment inside the binaries: {l:?}");
        } else {
            let (w, t) = parse_line(l);
            out.last_mut().expect("a word before the first # binary").1.push((w, t.to_string()));
        }
    }
    out
}

fn text_of(w: u64) -> Result<String, DescribeError> {
    let mut s = String::new();
    describe_word(w, &mut s)?;
    Ok(s)
}

/// Decode, re-encode, compare: the word back, or why not.
fn round_trip(w: u64) -> Result<Instr, String> {
    let i = decode(w).map_err(|e| format!("{e:?}"))?;
    let back = encode(&i).map_err(|e| format!("{i:?}: {e:?}"))?;
    if back != w {
        return Err(format!("{i:?}: re-encoded {back:#018x}"));
    }
    Ok(i)
}

/// The one board word the decoder refuses: MKVEC.v2i16 with bit 27 set.
/// Mesa 25.0.7 wrote it for `0x3C000000.h11` (its ISA.xml gave MKVEC's
/// sources a two-bit swizzle at 26..27 and 28..29); Arm's isa-v9-v14.xml
/// gives each a one-bit lane (`lane1` at 26, `lane0` at 28) and reserves
/// bits 27 and 29 as zero, and Mesa 26.2.1's ISA.xml reads one bit
/// (`<src lane="26">`), printing `.h1`. The board run cannot tell the
/// readings apart: in tri_varying's fragment shader (VA 7fffffffc100) both
/// take the high half of the constant, 0x3C00 (1.0, the alpha), and the
/// image read back exactly either way. The encoder never writes the bit,
/// so the decoder refuses the word rather than give back one it cannot
/// write; with the bit clear the instruction round trips.
const MKVEC_BIT27: u64 = 0x40a1_c100_0c05_db41;

/// Every board instruction decodes and re-encodes byte-equal (one named
/// exception), and our disassembly of every word is Mesa 26.2.1's.
#[test]
fn the_board_binaries_round_trip_and_read_as_mesa_26_2_1() {
    let b25 = binaries(BOARD_25);
    let b26 = binaries(BOARD_26);
    assert_eq!(b25.len(), BOARD_BINARIES, "binaries in the board fixture");
    assert_eq!(b26.len(), b25.len(), "the Mesa 26.2.1 fixture has the same binaries");
    let distinct: BTreeSet<Vec<u64>> = b25.iter().map(|(_, l)| l.iter().map(|x| x.0).collect()).collect();
    assert_eq!(distinct.len(), BOARD_DISTINCT_BINARIES, "distinct binaries");
    let (mut n, mut byte_equal, mut text_equal, mut refused) = (0, 0, 0, Vec::new());
    for ((label, l25), (label26, l26)) in b25.iter().zip(&b26) {
        assert_eq!(label, label26);
        assert_eq!(l25.len(), l26.len(), "{label}");
        for ((w, _), (w26, t26)) in l25.iter().zip(l26) {
            assert_eq!(w, w26, "{label}: the fixtures hold the same words");
            n += 1;
            match round_trip(*w) {
                Ok(_) => byte_equal += 1,
                Err(e) => refused.push((*w, e)),
            }
            assert_eq!(text_of(*w).as_deref(), Ok(t26.as_str()), "{label} {w:#018x}");
            text_equal += 1;
        }
    }
    assert_eq!(n, BOARD_INSTRUCTIONS, "instructions in the board fixture");
    assert_eq!(text_equal, n);
    assert_eq!(byte_equal, n - 1, "refused: {refused:?}");
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].0, MKVEC_BIT27);
    assert_eq!(decode(MKVEC_BIT27), Err(DecodeError::ReservedBitsSet { op: Op::MkvecV2i16, bits: 1 << 27 }));
    // With the bit clear, the same instruction round trips.
    let clean = MKVEC_BIT27 & !(1 << 27);
    assert!(round_trip(clean).is_ok());
}

/// What a Mesa 25.0.7 line becomes in Mesa 26.2.1's syntax, by the
/// differences between the two disassemblers (valhall/disasm.py, 25.0.7
/// against 26.2.1; presentation only, the words are the same), and which
/// rules a line needed.
fn mesa_25_to_26(w: u64, t25: &str, t26: &str) -> (String, BTreeSet<&'static str>) {
    let mut used = BTreeSet::new();
    // Split both into tokens on ", " and " ", keeping the separators.
    fn tokens(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        for c in s.chars() {
            if c == ' ' || c == ',' {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push(c.to_string());
            } else {
                cur.push(c);
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    }
    let a = tokens(t25);
    let b = tokens(t26);
    assert_eq!(a.len(), b.len(), "token counts differ:\n  25.0.7 {t25}\n  26.2.1 {t26}");
    // The operands the word's own print steps name, in order, with the
    // source or destination size in bits (None: a staging list or an
    // immediate). Token 0 is the mnemonic.
    let sizes: Vec<Option<u32>> = mesa_op(w)
        .unwrap_or_else(|| panic!("{w:#018x}: not an instruction of Mesa's ISA.xml"))
        .steps
        .iter()
        .filter_map(|s| match *s {
            S::Dest { size } | S::Src { size, .. } => Some(Some(size)),
            S::Staging { .. } | S::Imm { .. } => Some(None),
            _ => None,
        })
        .collect();
    let mut out = String::new();
    let mut operand = 0usize; // non-separator tokens seen
    for x in &a {
        let size = if x == " " || x == "," {
            None
        } else {
            operand += 1;
            operand.checked_sub(2).and_then(|k| sizes.get(k).copied().flatten())
        };
        let mut x = x.clone();
        // 1. The discard mark: 25.0.7 "^r5", 26.2.1 "r5^" (va_print_src).
        if let Some(rest) = x.strip_prefix("^r") {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            x = format!("r{digits}^{}", &rest[digits.len()..]);
            used.insert("discard mark after the register");
        }
        // 2. A 64-bit source or destination register: 25.0.7 one register,
        //    26.2.1 the pair "[r4^:r5^]".
        if size.is_some_and(|s| s > 32) && x.starts_with('r') && x[1..].starts_with(|c: char| c.is_ascii_digit()) {
            let r = x.trim_start_matches('r');
            let (num, mark) = r.split_at(r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len()));
            let n: u32 = num.parse().unwrap_or_else(|_| panic!("{t25}: {x}"));
            x = format!("[r{n}{mark}:r{}{mark}]", n + 1);
            used.insert("64-bit register pair");
        }
        // 3. A uniform: 25.0.7 "u5" (the 6-bit FAU index), 26.2.1 the
        //    64-bit slot and the word, "u2.w1" (".wN" only for 32 bits).
        if let Some(num) = x.strip_prefix('u') {
            if let Ok(n) = num.parse::<u32>() {
                x = if size.is_some_and(|s| s <= 32) {
                    format!("u{}.w{}", n >> 1, n & 1)
                } else {
                    format!("u{}", n >> 1)
                };
                used.insert("uniform as slot and word");
            }
        }
        out.push_str(&x);
    }
    (out, used)
}

/// The board's own text (Mesa 25.0.7) against Mesa 26.2.1's on the same
/// words: equal but for the three named differences of presentation, and
/// the MKVEC lane (the same bit read as two, see [`MKVEC_BIT27`]). Each
/// difference's line count is pinned.
#[test]
fn the_board_text_differs_from_mesa_26_2_1_only_by_the_named_rules() {
    let b25 = binaries(BOARD_25);
    let b26 = binaries(BOARD_26);
    let mut lines_per_rule: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut same, mut by_rules, mut mkvec) = (0, 0, 0);
    for ((_, l25), (_, l26)) in b25.iter().zip(&b26) {
        for ((w, t25), (_, t26)) in l25.iter().zip(l26) {
            if t25 == t26 {
                same += 1;
                continue;
            }
            if *w == MKVEC_BIT27 {
                assert_eq!(t25, "MKVEC.v2i16.wait0126 r1, ^r1.h00, 0x3C000000.h11");
                assert_eq!(t26, "MKVEC.v2i16.wait0126 r1, r1^.h0, 0x3C000000.h1");
                mkvec += 1;
                continue;
            }
            let (conv, used) = mesa_25_to_26(*w, t25, t26);
            assert_eq!(&conv, t26, "{w:#018x}: not explained by the named rules (25.0.7: {t25})");
            by_rules += 1;
            for r in used {
                *lines_per_rule.entry(r).or_default() += 1;
            }
        }
    }
    assert_eq!(same + by_rules + mkvec, BOARD_INSTRUCTIONS);
    assert_eq!((same, by_rules, mkvec), (33, 98, 1));
    let want: BTreeMap<&str, usize> =
        [("discard mark after the register", 96), ("64-bit register pair", 9), ("uniform as slot and word", 12)]
            .into_iter()
            .collect();
    assert_eq!(lines_per_rule, want);
}

/// Mesa's test vectors: our disassembly is the vector's text on every line;
/// every word but one named one decodes and re-encodes byte-equal.
#[test]
fn mesa_s_test_vectors_round_trip_and_read_as_mesa() {
    let mut n = 0;
    let mut refused = BTreeMap::new();
    for l in VECTORS.lines().filter(|l| !l.is_empty()) {
        let (w, t) = parse_line(l);
        n += 1;
        assert_eq!(text_of(w).as_deref(), Ok(t), "{l}");
        if let Err(e) = round_trip(w) {
            refused.insert(w, (t.to_string(), e));
        }
    }
    assert_eq!(n, VECTOR_LINES, "assembler-cases.txt lines");
    // ATOM1_RETURN.i32 with no staging register ("@,", sr_count 0): Arm's
    // XML fixes sr_count at count1 for ATOM1_RETURN (it returns a value),
    // so no v10 op has this word's opcode bits. A hand-written assembler
    // case (Mesa's assembler checks the syntax, not the encoding).
    let atom1 = 0x0869_8000_1800_0032u64;
    let want: BTreeSet<u64> = [atom1].into_iter().collect();
    assert_eq!(refused.keys().copied().collect::<BTreeSet<_>>(), want, "{refused:#?}");
    assert_eq!(decode(atom1), Err(DecodeError::Unknown { word: atom1 }));
}

/// The population: the generator emitted one op per `<instruction>` of the
/// XML valid for arch 10, counted here from the XML text itself.
#[test]
fn every_v10_instruction_of_the_xml_is_an_op() {
    let mut count = 0;
    let mut aliases = 0;
    for tag in KRAID_XML.split("<instruction ").skip(1) {
        let head = &tag[..tag.find('>').expect("an <instruction> tag ends")];
        let attr = |name: &str| -> Option<&str> {
            let k = format!(" {name}=\"");
            let s = format!(" {head}").find(&k)? + k.len() - 1;
            let rest = &head[s..];
            Some(&rest[..rest.find('"')?])
        };
        let since: u32 = attr("since").expect("since").parse().expect("a number");
        let until: u32 = attr("until").expect("until").parse().expect("a number");
        if since <= 10 && 10 <= until {
            count += 1;
            aliases += usize::from(attr("alias").is_some());
        }
    }
    assert_eq!(count, V10_VARIANTS, "v10 instructions in isa-v9-v14.xml");
    assert_eq!(VARIANTS, count, "ops the generator emitted");
    assert_eq!(OPS.len(), count);
    assert_eq!(OPS.iter().filter(|o| o.alias_of.is_some()).count(), aliases);
    assert_eq!(aliases, V10_ALIASES);
    for (i, (op, info)) in Op::ALL.iter().zip(OPS.iter()).enumerate() {
        assert_eq!(*op as usize, i);
        assert_eq!(info.op, *op);
    }
}

/// The values an operand's type and `restrict=` allow, the default first;
/// a plain number tries 0..64 (or its `min=`).
fn candidates(sym: &Sym, default: Option<u32>) -> Vec<u32> {
    let ok = |v: u32| check_value(sym, v).is_ok();
    let mut out: Vec<u32> = default.filter(|&d| ok(d)).into_iter().collect();
    match sym.kind {
        Kind::Enum(e) => out.extend(e.valid.iter().copied().filter(|&v| ok(v))),
        _ => out.extend(sym.range.map_or(0, |r| r.0)..64),
    }
    out
}

/// An instruction of `op` that encodes: every operand at its first
/// candidate, then, while it does not encode, each virtual operand through
/// its candidates (a virtual's value must be one its fields give back:
/// LOAD's `extend` is `none` whenever no lane is loaded). `Err` with the
/// first attempt's refusal if none encodes.
fn encodable(op: Op) -> Result<(Instr, u64), EncodeError> {
    let info = op.info();
    let mut i = Instr::new(op);
    for (j, o) in info.operands.iter().enumerate() {
        i.vals[j] = candidates(&info.syms[o.sym], o.default)[0];
    }
    let first = match encode(&i) {
        Ok(w) => return Ok((i, w)),
        Err(e) => e,
    };
    // Every combination of the virtual operands' candidates (two virtuals
    // can read each other's fields: LD_VAR_BUF's `update` and
    // `sample_position`).
    let virt: Vec<(usize, Vec<u32>)> = info
        .operands
        .iter()
        .enumerate()
        .filter(|(_, o)| info.syms[o.sym].field.is_none())
        .map(|(j, o)| (j, candidates(&info.syms[o.sym], o.default)))
        .collect();
    let mut at = vec![0usize; virt.len()];
    loop {
        for (k, (j, c)) in virt.iter().enumerate() {
            i.vals[*j] = c[at[k]];
        }
        if let Ok(w) = encode(&i) {
            return Ok((i, w));
        }
        let mut k = 0;
        loop {
            if k == virt.len() {
                return Err(first);
            }
            at[k] += 1;
            if at[k] < virt[k].1.len() {
                break;
            }
            at[k] = 0;
            k += 1;
        }
    }
}

/// Every op encodes an instruction of valid values into a word that decodes
/// back to it, as itself (`decode_as`) and, for a non-alias, as the answer
/// of `decode`; for an alias `decode` gives its base. Which of these words
/// Mesa's own ISA.xml has no instruction for is the second reading's gap,
/// pinned by name; every other word is described without an error.
#[test]
fn every_variant_encodes_and_decodes_back() {
    let mut not_in_mesa = Vec::new();
    let mut not_encoded = Vec::new();
    let mut not_described = Vec::new();
    for info in OPS.iter() {
        let (i, w) = match encodable(info.op) {
            Ok(x) => x,
            Err(e) => {
                not_encoded.push((info.mnemonic, e));
                continue;
            }
        };
        assert_eq!(decode_as(info.op, w), Ok(i), "{}", info.mnemonic);
        let d = decode(w).unwrap_or_else(|e| panic!("{}: {e:?}", info.mnemonic));
        assert_eq!(d.op, info.alias_of.unwrap_or(info.op), "{}", info.mnemonic);
        match text_of(w) {
            Ok(_) => {}
            Err(DescribeError::Unknown { .. }) => not_in_mesa.push(info.mnemonic),
            Err(e) => not_described.push((info.mnemonic, e)),
        }
    }
    assert_eq!(not_encoded, [], "ops with no encodable instruction");
    assert_eq!(not_described, [], "words Mesa names but does not describe");
    assert_eq!(not_in_mesa, NOT_IN_MESA_ISA_XML);
}

/// The ops whose encoded word no instruction of Mesa 26.2.1's ISA.xml names: the
/// rotates, the slice multiplies, the FPOW/FRCBRT/LOGB helpers, the carry
/// adds, the 64-bit multiply, DOORBELL, EUREKA, KABOOM and JUMP_EX, which
/// ISA.xml does not list.
const NOT_IN_MESA_ISA_XML: [&str; 50] = [
    "DOORBELL",
    "EUREKA",
    "FMUL_CSLICE",
    "FMUL_SLICE.f32",
    "FPOW_SC_APPLY",
    "FPOW_SC_DET.f32",
    "FRCBRT_APPROX_A.f32",
    "FRCBRT_APPROX_B.f32",
    "FRCBRT_APPROX_C.f32",
    "IADDC.u32",
    "ILOGB.f32",
    "ILOGB.v2f16",
    "IMUL.s64",
    "ISUBB.u32",
    "JUMP_EX",
    "KABOOM",
    "LOGB.f32",
    "LOGB.v2f16",
    "LROT.i32",
    "LROT.i64",
    "LROT.v2i16",
    "LROT.v4i8",
    "LROT_AND.i32",
    "LROT_AND.i64",
    "LROT_AND.v2i16",
    "LROT_AND.v4i8",
    "LROT_OR.i32",
    "LROT_OR.i64",
    "LROT_OR.v2i16",
    "LROT_OR.v4i8",
    "LROT_XOR.i32",
    "LROT_XOR.i64",
    "LROT_XOR.v2i16",
    "LROT_XOR.v4i8",
    "RROT.i32",
    "RROT.i64",
    "RROT.v2i16",
    "RROT.v4i8",
    "RROT_AND.i32",
    "RROT_AND.i64",
    "RROT_AND.v2i16",
    "RROT_AND.v4i8",
    "RROT_OR.i32",
    "RROT_OR.i64",
    "RROT_OR.v2i16",
    "RROT_OR.v4i8",
    "RROT_XOR.i32",
    "RROT_XOR.i64",
    "RROT_XOR.v2i16",
    "RROT_XOR.v4i8",
];

// ---- the refusals ---------------------------------------------------------

fn fadd() -> Instr {
    round_trip(0x00a4_c000_0000_0201).expect("FADD.f32 r0, r1, r2")
}

#[test]
fn the_encoder_refuses_a_value_that_does_not_fit() {
    let mut i = fadd();
    assert_eq!(i.op, Op::FaddF32);
    i.set("dest_register_index", 64).unwrap();
    assert_eq!(
        encode(&i),
        Err(EncodeError::Field {
            op: Op::FaddF32,
            field: "dest_register_index",
            why: Why::TooWide { value: 64, bits: 6 }
        })
    );
    let mut i = fadd();
    i.set("src1", 0x100).unwrap();
    assert!(matches!(encode(&i), Err(EncodeError::Field { field: "src1", why: Why::TooWide { .. }, .. })));
    let mut i = fadd();
    i.set("round", 7).unwrap();
    assert!(matches!(encode(&i), Err(EncodeError::Field { field: "round", why: Why::NotNamed { .. }, .. })));
    let mut i = fadd();
    i.vals[MAX_OPERANDS - 1] = 1;
    assert_eq!(encode(&i), Err(EncodeError::UnusedSlot { op: Op::FaddF32, slot: MAX_OPERANDS - 1 }));
}

/// `restrict=`, `align(n)` and `shr(n)`, on the ops that have them.
#[test]
fn the_encoder_refuses_restricted_misaligned_and_low_bits() {
    let first = |pred: &dyn Fn(&Sym) -> bool| -> (Op, &'static str) {
        for info in OPS.iter() {
            for o in info.operands {
                if pred(&info.syms[o.sym]) {
                    return (info.op, o.name);
                }
            }
        }
        panic!("no op has such an operand");
    };
    // A named slot value outside restrict="slot0 slot1 slot2".
    let (op, name) = first(&|s| s.name == "message_slot_index" && s.restrict.is_some());
    let mut i = encodable(op).expect("an encodable instruction").0;
    let s = &op.info().syms[op.info().operands.iter().find(|o| o.name == name).unwrap().sym];
    let Kind::Enum(e) = s.kind else { panic!() };
    let outside = *e.valid.iter().find(|v| !s.restrict.unwrap().contains(v)).expect("a value restrict leaves out");
    i.set(name, outside).unwrap();
    assert_eq!(encode(&i), Err(EncodeError::Field { op, field: name, why: Why::NotAllowed { value: outside } }));
    // align(4).
    let (op, name) = first(&|s| s.modifier == Some(Modifier::Align(4)));
    let mut i = encodable(op).expect("an encodable instruction").0;
    i.set(name, 6).unwrap();
    assert_eq!(encode(&i), Err(EncodeError::Field { op, field: name, why: Why::NotAligned { value: 6, align: 4 } }));
    // shr(3).
    let (op, name) = first(&|s| s.modifier == Some(Modifier::Shr(3)));
    let mut i = encodable(op).expect("an encodable instruction").0;
    i.set(name, 9).unwrap();
    assert_eq!(encode(&i), Err(EncodeError::Field { op, field: name, why: Why::LowBitsSet { value: 9, shift: 3 } }));
}

/// LD_ATTR_IMM's table index is a virtual operand: the field holds its low
/// four bits, and 12..15 read back as 60..63. A table index of 13 is
/// written as 13 and read back as 61: refused, not written.
#[test]
fn a_virtual_that_does_not_read_back_is_refused() {
    let (w, _) = parse_line("7c 7d 01 12 04 80 66 08    LD_ATTR_IMM");
    let mut i = round_trip(w).expect("the board's LD_ATTR_IMM");
    assert_eq!(i.op, Op::LdAttrImm);
    assert_eq!(i.get("attribute_table_index"), Some(1));
    i.set("attribute_table_index", 60).unwrap();
    let w60 = encode(&i).expect("table 60 is encoded as 12");
    assert_eq!((w60 >> 16) & 0xF, 12);
    i.set("attribute_table_index", 13).unwrap();
    assert_eq!(encode(&i), Err(EncodeError::ReadsBackAs { op: Op::LdAttrImm }));
}

/// The decoder: a computed field that does not hold its expression's value
/// (LD_ATTR_IMM's staging count, from its vector size) is refused.
#[test]
fn the_decoder_refuses_an_inconsistent_computed_field() {
    let (w, _) = parse_line("7c 7d 01 12 04 80 66 08    LD_ATTR_IMM");
    // sr_count (bits 33..35) is 2 for .v2.f32; make it 3.
    let bad = (w & !(7u64 << 33)) | (3u64 << 33);
    assert_eq!(
        decode(bad),
        Err(DecodeError::Inconsistent { op: Op::LdAttrImm, field: "sr_count", word: 3, expected: 2 })
    );
}

/// The `MOV_IMM` alias of `IADD_IMM`: `decode` gives the base, `decode_as`
/// the alias, and both encode the same word.
#[test]
fn an_alias_is_read_as_its_base_unless_asked() {
    let (w, _) = parse_line("c0 00 00 00 c7 c4 10 01    IADD_IMM.i32 r4, 0x0, #0xC7000000");
    let base = round_trip(w).expect("IADD_IMM");
    assert_eq!(base.op, Op::IaddImmI32);
    let alias = decode_as(Op::MovImmI32, w).expect("MOV_IMM.i32");
    assert_eq!(Op::MovImmI32.info().alias_of, Some(Op::IaddImmI32));
    assert_eq!(encode(&alias), Ok(w));
}

/// A text the gate names is checked against the code that writes it.
#[test]
fn describe_matches_describe_word() {
    let i = fadd();
    let mut s = String::new();
    describe(&i, &mut s).unwrap();
    assert_eq!(s, "FADD.f32 r0, r1, r2");
    let mut t = String::new();
    write!(t, "{i:?}").unwrap();
    assert!(t.starts_with("FADD.f32 "), "{t}");
}

/// A 64-bit source is an even register or uniform byte, unless it is a small
/// constant (mesa-26.2.1 kraid/encode_v9.rs:228-230: `encoded & 1 == 0`).
#[test]
fn a_64_bit_source_refuses_an_odd_register() {
    let mut i = Instr::new(Op::AcmpxchgI32);
    i.set("src0", 4).unwrap();
    let w = encode(&i).expect("ACMPXCHG.i32 src0 = r4");
    assert_eq!(decode(w).map(|d| d.get("src0")), Ok(Some(4)));
    for odd in [5u32, 0x41, 0x81, 0xE1] {
        let mut i = Instr::new(Op::AcmpxchgI32);
        i.set("src0", odd).unwrap();
        assert_eq!(
            encode(&i),
            Err(EncodeError::Field { op: Op::AcmpxchgI32, field: "src0", why: Why::OddSource64 { value: odd } }),
            "src0 = {odd:#x}"
        );
    }
    // A small constant may be odd.
    let mut i = Instr::new(Op::AcmpxchgI32);
    i.set("src0", 0xC1).unwrap();
    assert!(encode(&i).is_ok());
    // The decoder refuses the odd word as well.
    let mut i = Instr::new(Op::AcmpxchgI32);
    i.set("src0", 4).unwrap();
    let odd = (encode(&i).unwrap() & !0xFF) | 5;
    assert_eq!(
        decode(odd),
        Err(DecodeError::Field { op: Op::AcmpxchgI32, field: "src0", why: Why::OddSource64 { value: 5 } })
    );
}

/// Page 2 of the FAU reserves the special sources of 0xE0 and more (Mesa's
/// `reserved_page2`).
#[test]
fn fau_page_2_refuses_a_special_source_past_31() {
    let page2 = |src1: u32, page: u32| {
        let mut i = fadd();
        i.set("src1", src1).unwrap();
        i.set("fau_page_index", page).unwrap();
        i
    };
    let want = Err(EncodeError::Field { op: Op::FaddF32, field: "src1", why: Why::ReservedFauPage2 { value: 0xE0 } });
    assert_eq!(encode(&page2(0xE0, 2)), want);
    let w0 = encode(&page2(0xE0, 0)).expect("page 0 accepts 0xE0");
    assert!(encode(&page2(0xDF, 2)).is_ok(), "an immediate on page 2 is fine");
    // The raw word with the page set to 2.
    let w2 = w0 & !(3 << 57) | 2 << 57;
    assert_eq!(
        decode(w2),
        Err(DecodeError::Field { op: Op::FaddF32, field: "src1", why: Why::ReservedFauPage2 { value: 0xE0 } })
    );
    assert!(decode(w0).is_ok());
}

/// The numbers of [`builder_defaults_are_checked_against_the_corpus`]; the
/// doc comments of `Operand::default` and `Instr::new` quote them.
const DEFAULTS_TOTAL: usize = 970;
const DEFAULTS_EXERCISED: usize = 196;

/// Every word of the corpus (the board's binaries and Mesa's vectors) that
/// the decoder accepts.
fn corpus_words() -> Vec<u64> {
    let mut words: Vec<u64> = binaries(BOARD_25).iter().flat_map(|(_, l)| l.iter().map(|x| x.0)).collect();
    words.extend(VECTORS.lines().filter(|l| !l.is_empty()).map(|l| parse_line(l).0));
    words.sort_unstable();
    words.dedup();
    words.retain(|&w| decode(w).is_ok());
    words
}

/// A builder default (the XML's `default=`, taken by [`Instr::new`]) is
/// measured per operand of each op: the corpus decodes to an instruction;
/// the same op is built by `Instr::new` with only the operands whose value
/// is not their default set, and must encode to the same word. An operand
/// whose default never appears in the corpus is not measured by this: it is
/// counted and named unmeasured, not passed.
#[test]
fn builder_defaults_are_checked_against_the_corpus() {
    // (op, operand index) -> (words where the operand held its default).
    let mut held: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for w in corpus_words() {
        let d = decode(w).unwrap();
        let info = d.op.info();
        let mut b = Instr::new(d.op);
        for (i, o) in info.operands.iter().enumerate() {
            if o.default == Some(d.vals[i]) {
                *held.entry((d.op as usize, i)).or_default() += 1;
            } else {
                b.vals[i] = d.vals[i];
            }
        }
        b.ignored = d.ignored;
        assert_eq!(encode(&b), Ok(w), "{}: building with only the non-default operands set", info.mnemonic);
    }
    let total: usize = OPS.iter().map(|o| o.operands.iter().filter(|x| x.default.is_some()).count()).sum();
    let exercised = held.len();
    let never = total - exercised;
    println!("operand defaults: {total} in the table, {exercised} exercised by the corpus, {never} never exercised");
    assert_eq!(held.keys().filter(|&&(op, i)| OPS[op].operands[i].default.is_none()).count(), 0);
    assert_eq!((total, exercised), (DEFAULTS_TOTAL, DEFAULTS_EXERCISED), "re-derive the pin and the doc comments");
}
