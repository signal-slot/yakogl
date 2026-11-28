// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The register words of `src/vivante/{ts,early_z,pe_zs,pe_overwrite}.rs`
//! against Mesa 26.2.1's etnaviv headers.
//!
//! The headers are Mesa's generated `state.xml.h`, `state_3d.xml.h`,
//! `common.xml.h` and `common_3d.xml.h`, kept unchanged in
//! `driver/src/vivante/hw/` (README.md there). The vendored Mesa tree under
//! `third_party/mesa-26.2.1` holds no etnaviv header, so this is the copy the
//! driver itself builds against. They are read at test time and every `#define
//! NAME VALUE` line is parsed; a name missing from them fails the test.
//!
//! - `ts.rs` and `early_z.rs` keep their addresses and bit values as public
//!   constants (and `early_z` emits `PE_DEPTH_CONFIG` and `RA_EARLY_DEPTH`
//!   bits): both are compared.
//! - `pe_zs.rs` and `pe_overwrite.rs` hold no register value: they say which
//!   `PE_DEPTH_CONFIG` and `PE_COLOR_FORMAT` fields a draw needs. Their
//!   public functions are run, and the field each answer sets is placed in
//!   the header's word through `regs.rs` (the table `emit_pe_state` writes).
//!
//! ```text
//! cargo test --no-default-features --features gpu-vivante,shaderc-online --test gc_tiled_regs_mesa
//! ```

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online"))]

use std::collections::HashMap;
use yakogl::vivante::early_z::{self, early_z, ColorLayout, EarlyZInput, StencilFace};
use yakogl::vivante::pe_overwrite::{pe_color, PeTarget};
use yakogl::vivante::pe_zs::{pe_zs, ZsFunc, ZsInput};
use yakogl::vivante::regs::*;
use yakogl::vivante::ts::{self, *};

const HW: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/");

/// Every plain `#define NAME <number>` of Mesa's four headers.
fn header() -> HashMap<String, u64> {
    let mut m = HashMap::new();
    for f in ["state.xml.h", "state_3d.xml.h", "common.xml.h", "common_3d.xml.h"] {
        let text = std::fs::read_to_string(format!("{HW}{f}")).unwrap_or_else(|e| panic!("{f}: {e}"));
        for line in text.lines() {
            let Some(rest) = line.trim_start().strip_prefix("#define") else { continue };
            let mut it = rest.split_whitespace();
            let (Some(name), Some(val)) = (it.next(), it.next()) else { continue };
            // an indexed register, `NAME(i0) (0xBASE + 0x4*(i0))`: element 0's address
            let (name, val) = match (name.split_once('('), val.strip_prefix("(0x")) {
                (Some((n, _)), Some(b)) => (n, format!("0x{}", b.trim_end_matches(|c: char| !c.is_ascii_hexdigit()))),
                _ => (name, val.to_string()),
            };
            let val = val.as_str();
            let v = if let Some(h) = val.strip_prefix("0x") { u64::from_str_radix(h, 16) } else { val.parse() };
            if let Ok(v) = v {
                m.insert(name.to_string(), v);
            }
        }
    }
    m
}

struct H(HashMap<String, u64>);

impl H {
    fn get(&self, name: &str) -> u32 {
        *self.0.get(name).unwrap_or_else(|| panic!("{name} is not a plain #define in Mesa's headers")) as u32
    }
    /// `ours` must equal the header's `name`.
    fn eq(&self, what: &str, ours: u32, name: &str) {
        assert_eq!(ours, self.get(name), "{what}: ours {ours:#010x} vs {name}");
    }
}

fn h() -> H {
    H(header())
}

#[test]
fn ts_rs_addresses_and_values() {
    let h = h();
    h.eq("PE_MEM_CONFIG", ts::PE_MEM_CONFIG, "VIVS_PE_MEM_CONFIG");
    h.eq("TS_FLUSH_CACHE", ts::TS_FLUSH_CACHE, "VIVS_TS_FLUSH_CACHE");
    h.eq("TS_MEM_CONFIG", ts::TS_MEM_CONFIG, "VIVS_TS_MEM_CONFIG");
    h.eq("RS_CLEAR_CONTROL", ts::RS_CLEAR_CONTROL, "VIVS_RS_CLEAR_CONTROL");
    h.eq("TS_COLOR_STATUS_BASE", TS_COLOR_STATUS_BASE, "VIVS_TS_COLOR_STATUS_BASE");
    h.eq("TS_COLOR_SURFACE_BASE", TS_COLOR_SURFACE_BASE, "VIVS_TS_COLOR_SURFACE_BASE");
    h.eq("TS_COLOR_CLEAR_VALUE", TS_COLOR_CLEAR_VALUE, "VIVS_TS_COLOR_CLEAR_VALUE");
    h.eq("TS_COLOR_AUTO_DISABLE_COUNT", TS_COLOR_AUTO_DISABLE_COUNT, "VIVS_TS_COLOR_AUTO_DISABLE_COUNT");
    h.eq("TS_COLOR_CLEAR_VALUE_EXT", TS_COLOR_CLEAR_VALUE_EXT, "VIVS_TS_COLOR_CLEAR_VALUE_EXT");
    h.eq("GL_FLUSH_CACHE", GL_FLUSH_CACHE, "VIVS_GL_FLUSH_CACHE");
    h.eq("GL_SEMAPHORE_TOKEN", GL_SEMAPHORE_TOKEN, "VIVS_GL_SEMAPHORE_TOKEN");
    h.eq("GL_STALL_TOKEN", GL_STALL_TOKEN, "VIVS_GL_STALL_TOKEN");
    h.eq("RS_KICKER", RS_KICKER, "VIVS_RS_KICKER");
    h.eq("RS_CONFIG", RS_CONFIG, "VIVS_RS_CONFIG");
    h.eq("RS_SOURCE_STRIDE", RS_SOURCE_STRIDE, "VIVS_RS_SOURCE_STRIDE");
    h.eq("RS_DEST_STRIDE", RS_DEST_STRIDE, "VIVS_RS_DEST_STRIDE");
    h.eq("RS_WINDOW_SIZE", RS_WINDOW_SIZE, "VIVS_RS_WINDOW_SIZE");
    // the indexed registers: element 0's address
    h.eq("RS_DITHER0", RS_DITHER0, "VIVS_RS_DITHER");
    h.eq("RS_FILL_VALUE0", RS_FILL_VALUE0, "VIVS_RS_FILL_VALUE");
    h.eq("RS_EXTRA_CONFIG", RS_EXTRA_CONFIG, "VIVS_RS_EXTRA_CONFIG");
    h.eq("RS_SINGLE_BUFFER", RS_SINGLE_BUFFER, "VIVS_RS_SINGLE_BUFFER");
    h.eq("RS_PIPE_SOURCE_ADDR0", RS_PIPE_SOURCE_ADDR0, "VIVS_RS_PIPE_SOURCE_ADDR");
    h.eq("RS_PIPE_DEST_ADDR0", RS_PIPE_DEST_ADDR0, "VIVS_RS_PIPE_DEST_ADDR");
    h.eq("RS_PIPE_OFFSET0", RS_PIPE_OFFSET0, "VIVS_RS_PIPE_OFFSET");
}

#[test]
fn ts_bit_fields_and_enums() {
    let h = h();
    // PE_MEM_CONFIG: mask and shift of the two TS-mode fields
    h.eq("color TS mode shift", PE_MEM_CONFIG_COLOR_TS_MODE_SHIFT, "VIVS_PE_MEM_CONFIG_COLOR_TS_MODE__SHIFT");
    h.eq("color TS mode mask", PE_MEM_CONFIG_COLOR_TS_MODE_MASK, "VIVS_PE_MEM_CONFIG_COLOR_TS_MODE__MASK");
    h.eq("depth TS mode shift", PE_MEM_CONFIG_DEPTH_TS_MODE_SHIFT, "VIVS_PE_MEM_CONFIG_DEPTH_TS_MODE__SHIFT");
    h.eq("depth TS mode mask", PE_MEM_CONFIG_DEPTH_TS_MODE_MASK, "VIVS_PE_MEM_CONFIG_DEPTH_TS_MODE__MASK");
    // TS_MEM_CONFIG bits
    h.eq("DEPTH_FAST_CLEAR", TS_MEM_DEPTH_FAST_CLEAR, "VIVS_TS_MEM_CONFIG_DEPTH_FAST_CLEAR");
    h.eq("COLOR_FAST_CLEAR", TS_MEM_COLOR_FAST_CLEAR, "VIVS_TS_MEM_CONFIG_COLOR_FAST_CLEAR");
    h.eq("DEPTH_16BPP", TS_MEM_DEPTH_16BPP, "VIVS_TS_MEM_CONFIG_DEPTH_16BPP");
    h.eq("DEPTH_AUTO_DISABLE", TS_MEM_DEPTH_AUTO_DISABLE, "VIVS_TS_MEM_CONFIG_DEPTH_AUTO_DISABLE");
    h.eq("COLOR_AUTO_DISABLE", TS_MEM_COLOR_AUTO_DISABLE, "VIVS_TS_MEM_CONFIG_COLOR_AUTO_DISABLE");
    h.eq("64BPP_FORMAT", TS_MEM_64BPP_FORMAT, "VIVS_TS_MEM_CONFIG_64BPP_FORMAT");
    h.eq("COLOR_COMPRESSION", TS_MEM_COLOR_COMPRESSION, "VIVS_TS_MEM_CONFIG_COLOR_COMPRESSION");
    // COMPRESSION_FORMAT_3: the value 3 in the 4-bit field at bit 8
    let fmt_shift = h.get("VIVS_TS_MEM_CONFIG_COLOR_COMPRESSION_FORMAT__SHIFT");
    let fmt_mask = h.get("VIVS_TS_MEM_CONFIG_COLOR_COMPRESSION_FORMAT__MASK");
    assert_eq!(TS_MEM_COLOR_COMPRESSION_FORMAT_3, (3 << fmt_shift) & fmt_mask);
    // RS_CLEAR_CONTROL: bits field and mode
    h.eq("RS_CLEAR_BITS_MASK", RS_CLEAR_BITS_MASK, "VIVS_RS_CLEAR_CONTROL_BITS__MASK");
    h.eq("RS_CLEAR_MODE_ENABLED1", RS_CLEAR_MODE_ENABLED1, "VIVS_RS_CLEAR_CONTROL_MODE_ENABLED1");
    // TS modes (the enum of common_3d.xml.h)
    h.eq("TS_MODE_128B", TS_MODE_128B, "TS_MODE_128B");
    h.eq("TS_MODE_256B", TS_MODE_256B, "TS_MODE_256B");
    // TS_FLUSH_CACHE, GL_FLUSH_CACHE (COLOR | DEPTH), RS and stall words
    h.eq("TS_FLUSH", TS_FLUSH, "VIVS_TS_FLUSH_CACHE_FLUSH");
    // (COLOR is 2 and DEPTH 1: the word is their union)
    assert_eq!(GL_FLUSH_COLOR_DEPTH, h.get("VIVS_GL_FLUSH_CACHE_COLOR") | h.get("VIVS_GL_FLUSH_CACHE_DEPTH"));
    // STALL_RA_PE: FROM = RA (bits 0..5), TO = PE (bits 8..13)
    let from = (h.get("SYNC_RECIPIENT_RA") << h.get("VIVS_GL_STALL_TOKEN_FROM__SHIFT"))
        & h.get("VIVS_GL_STALL_TOKEN_FROM__MASK");
    let to =
        (h.get("SYNC_RECIPIENT_PE") << h.get("VIVS_GL_STALL_TOKEN_TO__SHIFT")) & h.get("VIVS_GL_STALL_TOKEN_TO__MASK");
    assert_eq!(STALL_RA_PE, from | to);
    h.eq("RS_SINGLE_BUFFER_ENABLE", RS_SINGLE_BUFFER_ENABLE, "VIVS_RS_SINGLE_BUFFER_ENABLE");
    // RS_CONFIG
    h.eq("RS_CONFIG_SOURCE_TILED", RS_CONFIG_SOURCE_TILED, "VIVS_RS_CONFIG_SOURCE_TILED");
    h.eq("RS_CONFIG_DEST_TILED", RS_CONFIG_DEST_TILED, "VIVS_RS_CONFIG_DEST_TILED");
    h.eq("RS_FORMAT_A8R8G8B8", RS_FORMAT_A8R8G8B8, "RS_FORMAT_A8R8G8B8");
    // RS strides: the source and destination words share bit positions
    h.eq("RS_STRIDE_TILING", RS_STRIDE_TILING, "VIVS_RS_SOURCE_STRIDE_TILING");
    h.eq("RS_STRIDE_TILING (dest)", RS_STRIDE_TILING, "VIVS_RS_DEST_STRIDE_TILING");
    h.eq("RS_STRIDE_SUPER_TILED_NEW", RS_STRIDE_SUPER_TILED_NEW, "VIVS_RS_SOURCE_STRIDE_SUPER_TILED_NEW");
    h.eq("RS_STRIDE_SUPER_TILED_NEW (dest)", RS_STRIDE_SUPER_TILED_NEW, "VIVS_RS_DEST_STRIDE_SUPER_TILED_NEW");
    h.eq("RS_SOURCE_STRIDE_TS_MODE", RS_SOURCE_STRIDE_TS_MODE, "VIVS_RS_SOURCE_STRIDE_TS_MODE__MASK");
    // the composed probe words
    assert_eq!(
        PROBE_RESOLVE_CONFIG,
        (h.get("RS_FORMAT_A8R8G8B8") << h.get("VIVS_RS_CONFIG_SOURCE_FORMAT__SHIFT"))
            | h.get("VIVS_RS_CONFIG_SOURCE_TILED")
            | (h.get("RS_FORMAT_A8R8G8B8") << h.get("VIVS_RS_CONFIG_DEST_FORMAT__SHIFT"))
    );
    assert_eq!(
        PROBE_FILL_CONFIG,
        (h.get("RS_FORMAT_A8R8G8B8") << h.get("VIVS_RS_CONFIG_SOURCE_FORMAT__SHIFT"))
            | (h.get("RS_FORMAT_A8R8G8B8") << h.get("VIVS_RS_CONFIG_DEST_FORMAT__SHIFT"))
            | h.get("VIVS_RS_CONFIG_DEST_TILED")
    );
}

/// The words ts.rs builds, through its public functions.
#[test]
fn ts_words_through_the_api() {
    let h = h();
    let c = h.get("VIVS_PE_MEM_CONFIG_COLOR_TS_MODE__MASK");
    let d = h.get("VIVS_PE_MEM_CONFIG_DEPTH_TS_MODE__MASK");
    // a one-bit field: mode 1 sets exactly the mask, mode 0 nothing
    assert_eq!(pe_mem_config(Some(TS_MODE_256B), None), c);
    assert_eq!(pe_mem_config(None, Some(TS_MODE_256B)), d);
    assert_eq!(pe_mem_config(Some(TS_MODE_256B), Some(TS_MODE_256B)), c | d);
    assert_eq!(pe_mem_config(Some(TS_MODE_128B), Some(TS_MODE_128B)), 0);
    assert_eq!(pe_mem_config(None, None), 0);

    let one = |f: fn(&mut TsMemConfig)| {
        let mut t = TsMemConfig::default();
        f(&mut t);
        ts_mem_config(t)
    };
    assert_eq!(one(|t| t.color_64bpp = true), h.get("VIVS_TS_MEM_CONFIG_64BPP_FORMAT"));
    assert_eq!(one(|t| t.depth_16bpp = true), h.get("VIVS_TS_MEM_CONFIG_DEPTH_16BPP"));
    assert_eq!(one(|t| t.color_valid = true), h.get("VIVS_TS_MEM_CONFIG_COLOR_FAST_CLEAR"));
    assert_eq!(one(|t| t.depth_valid = true), h.get("VIVS_TS_MEM_CONFIG_DEPTH_FAST_CLEAR"));
    assert_eq!(one(|t| t.color_auto_disable = true), h.get("VIVS_TS_MEM_CONFIG_COLOR_AUTO_DISABLE"));
    assert_eq!(one(|t| t.depth_auto_disable = true), h.get("VIVS_TS_MEM_CONFIG_DEPTH_AUTO_DISABLE"));

    // RS_CLEAR_CONTROL: MODE_ENABLED1 | the 16 clear bits
    let mode1 = h.get("VIVS_RS_CLEAR_CONTROL_MODE_ENABLED1");
    let bits = h.get("VIVS_RS_CLEAR_CONTROL_BITS__MASK");
    assert_eq!(rs_clear_control(0xeeee), mode1 | 0xeeee);
    assert_eq!(rs_clear_control(0xffff_ffff), mode1 | bits);
}

/// early_z.rs and the regs.rs bits it assembles.
#[test]
fn early_z_bits() {
    let h = h();
    h.eq(
        "RA_EARLY_DEPTH_LATE_DEPTH_MSAA",
        early_z::RA_EARLY_DEPTH_LATE_DEPTH_MSAA,
        "VIVS_RA_EARLY_DEPTH_LATE_DEPTH_MSAA",
    );
    h.eq("RA_EARLY_DEPTH", GC_RA_EARLY_DEPTH, "VIVS_RA_EARLY_DEPTH");
    h.eq("TEST_ENABLE", GC_RA_EARLY_DEPTH_TEST_ENABLE, "VIVS_RA_EARLY_DEPTH_TEST_ENABLE");
    h.eq("FORWARD_Z", GC_RA_EARLY_DEPTH_FORWARD_Z, "VIVS_RA_EARLY_DEPTH_FORWARD_Z");
    h.eq("FORWARD_W", GC_RA_EARLY_DEPTH_FORWARD_W, "VIVS_RA_EARLY_DEPTH_FORWARD_W");
    h.eq("WRITE_DISABLE", GC_RA_EARLY_DEPTH_WRITE_DISABLE, "VIVS_RA_EARLY_DEPTH_WRITE_DISABLE");
    h.eq("PE_DEPTH_CONFIG", GC_PE_DEPTH_CONFIG, "VIVS_PE_DEPTH_CONFIG");
    h.eq("EARLY_Z", GC_PE_DEPTH_EARLY_Z, "VIVS_PE_DEPTH_CONFIG_EARLY_Z");
    h.eq("DISABLE_ZS", GC_PE_DEPTH_DISABLE_ZS, "VIVS_PE_DEPTH_CONFIG_DISABLE_ZS");
    h.eq("WRITE_ENABLE", GC_PE_DEPTH_WRITE_ENABLE, "VIVS_PE_DEPTH_CONFIG_WRITE_ENABLE");
    h.eq("SUPER_TILED", GC_PE_DEPTH_SUPER_TILED, "VIVS_PE_DEPTH_CONFIG_SUPER_TILED");
    h.eq("DEPTH_MODE_NONE", GC_PE_DEPTH_MODE_NONE, "VIVS_PE_DEPTH_CONFIG_DEPTH_MODE_NONE");
    h.eq("DEPTH_MODE_Z", GC_PE_DEPTH_MODE_Z, "VIVS_PE_DEPTH_CONFIG_DEPTH_MODE_Z");
    h.eq("FORMAT_D16", GC_PE_DEPTH_FORMAT_D16, "VIVS_PE_DEPTH_CONFIG_DEPTH_FORMAT_D16");
    h.eq("FORMAT_D24S8", GC_PE_DEPTH_FORMAT_D24S8, "VIVS_PE_DEPTH_CONFIG_DEPTH_FORMAT_D24S8");
    // DEPTH_FUNC: 3 bits at bit 8
    let fshift = h.get("VIVS_PE_DEPTH_CONFIG_DEPTH_FUNC__SHIFT");
    let fmask = h.get("VIVS_PE_DEPTH_CONFIG_DEPTH_FUNC__MASK");
    for f in 0..8 {
        assert_eq!(pe_depth_func(f), (f << fshift) & fmask);
    }
    // the base bits are a blob value: they must not collide with a named bit
    let named = h.get("VIVS_RA_EARLY_DEPTH_TEST_ENABLE")
        | h.get("VIVS_RA_EARLY_DEPTH_FORWARD_Z")
        | h.get("VIVS_RA_EARLY_DEPTH_FORWARD_W")
        | h.get("VIVS_RA_EARLY_DEPTH_WRITE_DISABLE")
        | h.get("VIVS_RA_EARLY_DEPTH_LATE_DEPTH_MSAA");
    assert_eq!(GC_RA_EARLY_DEPTH_BASE & named, 0);

    // the emitted words, from a tiled depth-writing LEQUAL draw with early depth
    let on = EarlyZInput {
        depth_enabled: true,
        depth_func_always: false,
        depth_write: true,
        ..EarlyZInput::gc7000(ColorLayout::Tiled)
    };
    let e = early_z(&on);
    assert!(e.early_test && e.early_write);
    assert_eq!(e.pe_bits, h.get("VIVS_PE_DEPTH_CONFIG_EARLY_Z") | h.get("VIVS_PE_DEPTH_CONFIG_DISABLE_ZS"));
    assert_eq!(
        e.ra_word,
        GC_RA_EARLY_DEPTH_BASE
            | h.get("VIVS_RA_EARLY_DEPTH_TEST_ENABLE")
            | h.get("VIVS_RA_EARLY_DEPTH_FORWARD_W")
            | h.get("VIVS_RA_EARLY_DEPTH_FORWARD_Z")
    );
    // no early write: WRITE_DISABLE joins; late depth with MSAA: LATE_DEPTH_MSAA
    let late = early_z(&EarlyZInput { writes_z: true, msaa: true, ..on });
    assert!(late.late_write && late.late_test);
    assert_eq!(late.pe_bits, 0);
    assert_eq!(
        late.ra_word,
        GC_RA_EARLY_DEPTH_BASE
            | h.get("VIVS_RA_EARLY_DEPTH_FORWARD_W")
            | h.get("VIVS_RA_EARLY_DEPTH_FORWARD_Z")
            | h.get("VIVS_RA_EARLY_DEPTH_WRITE_DISABLE")
            | h.get("VIVS_RA_EARLY_DEPTH_LATE_DEPTH_MSAA")
    );
    // a linear target (the backend's draw_early_z): no early test, ZS still needed only when used
    let lin = early_z::draw_early_z(false);
    assert_eq!(lin.pe_bits, h.get("VIVS_PE_DEPTH_CONFIG_DISABLE_ZS"));
    // stencil-only draw: late stencil, no DISABLE_ZS
    let f = StencilFace { enabled: true, func_always: false, ops_keep: true };
    let st = early_z(&EarlyZInput { stencil: [f, f], writes_z: true, ..EarlyZInput::gc7000(ColorLayout::Tiled) });
    assert_eq!(st.pe_bits & h.get("VIVS_PE_DEPTH_CONFIG_DISABLE_ZS"), 0);
}

/// pe_zs.rs: the fields of PE_DEPTH_CONFIG its answer selects.
#[test]
fn pe_zs_fields() {
    let h = h();
    let none = pe_zs(ZsInput::default());
    assert!(none.disable_zs && !none.depth_write && none.depth_func == ZsFunc::Always);
    // the word the backend builds for "no attachment": DISABLE_ZS, mode NONE
    assert_eq!(
        GC_PE_DEPTH_MODE_NONE | if none.disable_zs { GC_PE_DEPTH_DISABLE_ZS } else { 0 },
        h.get("VIVS_PE_DEPTH_CONFIG_DEPTH_MODE_NONE") | h.get("VIVS_PE_DEPTH_CONFIG_DISABLE_ZS")
    );
    let draw = pe_zs(ZsInput { depth_test: true, depth_write: true, has_depth_attachment: true, ..Default::default() });
    assert!(!draw.disable_zs && draw.depth_write && draw.depth_func == ZsFunc::Gl);
    assert_eq!(if draw.depth_write { GC_PE_DEPTH_WRITE_ENABLE } else { 0 }, h.get("VIVS_PE_DEPTH_CONFIG_WRITE_ENABLE"));
    // a depth clear writes depth, tests ALWAYS (func 0 here is the GLES ALWAYS slot of the bit field)
    let clr = pe_zs(ZsInput { is_clear: true, clear_depth: true, has_depth_attachment: true, ..Default::default() });
    assert!(clr.depth_write && !clr.disable_zs && clr.depth_func == ZsFunc::Always);
}

/// pe_overwrite.rs: COMPONENTS (4 bits at bit 8, R = bit 0) and OVERWRITE.
#[test]
fn pe_overwrite_fields() {
    let h = h();
    let shift = h.get("VIVS_PE_COLOR_FORMAT_COMPONENTS__SHIFT");
    let mask = h.get("VIVS_PE_COLOR_FORMAT_COMPONENTS__MASK");
    assert_eq!(mask >> shift, 0xf, "COMPONENTS is a 4-bit field");
    h.eq("PE_COLOR_FORMAT", GC_PE_COLOR_FORMAT, "VIVS_PE_COLOR_FORMAT");
    h.eq("OVERWRITE", GC_PE_COLOR_FORMAT_OVERWRITE, "VIVS_PE_COLOR_FORMAT_OVERWRITE");
    for gl_mask in 0..16u32 {
        for blend in [false, true] {
            for alpha_bits in [0, 8] {
                for target in [PeTarget::Window, PeTarget::Fbo] {
                    for msaa in [false, true] {
                        let c = pe_color(gl_mask, blend, alpha_bits, target, msaa);
                        assert!(c.components <= 0xf);
                        // as regs.rs places it, the field is the header's
                        assert_eq!(pe_color_format_components(c.components), (c.components << shift) & mask);
                        if c.overwrite {
                            // OVERWRITE with a partial mask would write channels the GL masked
                            assert_eq!(c.components, 0xf);
                        }
                    }
                }
            }
        }
    }
    // R is bit 0 .. A bit 3: an RGBA mask passes through unchanged for an alpha target
    let c = pe_color(0b0101, false, 8, PeTarget::Fbo, false);
    assert_eq!(pe_color_format_components(c.components), 0b0101 << shift);
    let full = pe_color(0xf, false, 8, PeTarget::Fbo, false);
    assert!(full.overwrite);
    assert_eq!(GC_PE_COLOR_FORMAT_OVERWRITE, h.get("VIVS_PE_COLOR_FORMAT_OVERWRITE"));
}
