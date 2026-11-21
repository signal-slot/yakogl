// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Render target 3 in the control lists of the real V3D C job builder
//! (v3d-frame4-test, plan #368 RC14): `driver/src/v3d/v3d_job.c` builds a job
//! with four colour render targets and this test decodes what it wrote, the
//! binning list's TILE_BINNING_MODE_CFG, the render list's tile rendering mode
//! packets, and the generic per-tile list's loads and stores, and compares RT 3's
//! words with Mesa 26.2.1.
//!
//! What it adds to tests/es3_four_rt_v3d.rs: that file pins the tile size the
//! job asks `v3d_frame_tiling_*` for (1-4 RTs, bpp); this one reads the packets
//! themselves for RT 3 and does not repeat the tiling rule. Pinned here:
//!
//! - TILE_RENDERING_MODE_CFG_COLOR, RT 3's internal bpp, internal type and clamp,
//!   with RT 0-2 unchanged; the RT count and maximum bpp in the binning and the
//!   rendering mode (Common) packets (RT 3 alone can be the 128-bpp one);
//! - TILE_RENDERING_MODE_CFG_CLEAR_COLORS part 1 (and part 2 at 64 bpp, part 3
//!   at 128) for RT 3, the 128-bit clear colour split as Mesa splits it;
//! - the LOAD_TILE_BUFFER_GENERAL of RT 3 when it is not cleared, and none when
//!   it is, or when it has no buffer (GL_NONE);
//! - the STORE_TILE_BUFFER_GENERAL of RT 3 at the end of each tile, after RT 0-2;
//! - a draw's BLEND_ENABLES and BLEND_CFG, which name RT 3 with RT 0-2 (Mesa's
//!   blend_enables is (1 << max_rts) - 1 when RT 0 blends without independent
//!   blend, v3dx_state.c:148-156, emitted by v3dx_emit.c:525-568).
//!
//! Formats on RT 3: RGBA8 (32 bpp), RGBA16F (64), RG32F (64), RGBA32UI (128)
//! and RGBA32F (128); the two 32F rows of v3d_job.c's `rt_internal` are
//! v3d-frame4's (v3dx_format_table.c:319-326).
//!
//! Where the expected values come from. Packet layouts are read at run time
//! from Mesa's own `v3d_packet.xml` (third_party/mesa-26.2.1/src/broadcom/cle/,
//! the 4.2 packets; `start` counts from the byte after the opcode), so no bit
//! position is typed in here. The internal type and bpp per output image format
//! are `v3dX(get_internal_type_bpp_for_output_format)`
//! (src/gallium/drivers/v3d/v3dx_format_table.c:220-341). v3dx_rcl.c, which
//! holds the clear colour split, the clamp rule and the load/store order, is not
//! vendored in third_party/; those rows cite the Mesa 26.2.1 lines the C port
//! names in its comments (v3d_job.c, "v3dx_rcl.c:833-860", ":898-913",
//! ":396-407", ":198-212", ":258-283") and the XML for the enum values.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_frame_rt4
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use yakogl::v3d::ffi::*;

/* ---- Mesa's packet definitions, read from v3d_packet.xml ---- */

const XML: &str = "third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml";

struct Pkt {
    code: u8,
    name: String,
    /// The `sub-id` field's default (the `Tile Rendering Mode Cfg` family), if any.
    sub_id: Option<u8>,
    fields: Vec<(String, u32, u32)>,
    /// Bytes with the opcode byte.
    size: usize,
}

fn attr(line: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let at = line.find(&key)? + key.len();
    let end = line[at..].find('"')?;
    Some(line[at..at + end].to_string())
}

/// The V3D 4.2 packets: those with no `min_ver` above 42 (the 7.1 twins share
/// codes and names).
fn packets() -> &'static [Pkt] {
    static P: OnceLock<Vec<Pkt>> = OnceLock::new();
    P.get_or_init(|| {
        let text = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(XML))
            .expect("Mesa's v3d_packet.xml");
        let mut out: Vec<Pkt> = Vec::new();
        let mut cur: Option<Pkt> = None;
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with("<packet ") {
                let v71 = attr(l, "min_ver").is_some_and(|v| v.parse::<u32>().unwrap() > 42);
                let p = Pkt {
                    code: attr(l, "code").map(|c| c.parse().unwrap()).unwrap_or(0),
                    name: attr(l, "name").unwrap(),
                    sub_id: None,
                    fields: Vec::new(),
                    size: 1,
                };
                if l.ends_with("/>") {
                    if !v71 {
                        out.push(p);
                    }
                } else if v71 {
                    /* a 7.1 packet: its fields are read into a throwaway */
                    cur = Some(Pkt { code: 255, ..p });
                } else {
                    cur = Some(p);
                }
            } else if l.starts_with("<field ") {
                if let Some(p) = cur.as_mut() {
                    let name = attr(l, "name").unwrap();
                    let size: u32 = attr(l, "size").unwrap().parse().unwrap();
                    let start: u32 = attr(l, "start").unwrap().parse().unwrap();
                    if name == "sub-id" {
                        p.sub_id = attr(l, "default").map(|d| d.parse().unwrap());
                    }
                    p.size = p.size.max(1 + (start + size).div_ceil(8) as usize);
                    p.fields.push((name, start, size));
                }
            } else if l.starts_with("</packet>") {
                if let Some(p) = cur.take() {
                    if p.code != 255 {
                        out.push(p);
                    }
                }
            }
        }
        out
    })
}

fn pkt(name: &str) -> &'static Pkt {
    let mut it = packets().iter().filter(|p| p.name == name);
    let p = it.next().unwrap_or_else(|| panic!("{XML}: no 4.2 packet \"{name}\""));
    assert!(it.next().is_none(), "{XML}: \"{name}\" twice");
    p
}

/// One decoded packet: its Mesa name and the bytes after the opcode.
#[derive(Clone, Debug)]
struct Dec {
    name: &'static str,
    data: Vec<u8>,
}

impl Dec {
    /// Field `field` of this packet, bit `start` counted from the byte after the opcode.
    fn get(&self, field: &str) -> u64 {
        let p = pkt(self.name);
        let &(_, start, size) = p
            .fields
            .iter()
            .find(|f| f.0 == field)
            .unwrap_or_else(|| panic!("{XML}: \"{}\" has no field \"{field}\"", self.name));
        let mut v: u128 = 0;
        for (i, b) in self.data.iter().enumerate().take(16) {
            v |= (*b as u128) << (8 * i);
        }
        ((v >> start) & ((1u128 << size) - 1)) as u64
    }
}

/// Walk a control list from its opcode bytes: the packet sizes are the XML's.
fn decode(bytes: &[u8]) -> Vec<Dec> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let code = bytes[at];
        let cands: Vec<&Pkt> = packets().iter().filter(|p| p.code == code).collect();
        assert!(!cands.is_empty(), "opcode {code} at byte {at}: not a 4.2 packet in {XML}");
        let size = cands[0].size;
        for c in &cands {
            assert_eq!(c.size, size, "{XML}: opcode {code} has packets of different sizes");
        }
        let p = if cands.len() == 1 {
            cands[0]
        } else {
            let sub = bytes[at + 1] & 0xf;
            cands.iter().find(|c| c.sub_id == Some(sub)).unwrap_or_else(|| panic!("opcode {code} sub-id {sub}"))
        };
        assert!(at + size <= bytes.len(), "packet \"{}\" runs past the list", p.name);
        out.push(Dec { name: &p.name, data: bytes[at + 1..at + size].to_vec() });
        at += size;
    }
    out
}

/* ---- the C job builder ---- */

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_job.c` and v3d_frame.c, built as a shared object of this
/// test's own and reached through the handle only (RTLD_LOCAL).
fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_frame_rt4");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_frame_rt4-{}.so", std::process::id()));
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args([
                "driver/src/v3d/v3d_job.c",
                "driver/src/v3d/v3d_frame.c",
                "driver/src/v3d/v3d_tex.c",
                "driver/src/v3d/v3d_tiling_mesa.c",
                "-lm",
                "-o",
            ])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "driver/src/v3d/v3d_job.c builds");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the job builder loads");
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in v3d_job.c");
    p
}

type BeginFn = unsafe extern "C" fn(*mut V3dJob, *const V3dJobTarget, *mut V3dCl, *mut V3dCl, *mut V3dCl) -> bool;
type EndFn = unsafe extern "C" fn(*mut V3dJob, *const V3dJobClear, *mut u32, *mut u32) -> bool;

const BCL_GPU: u32 = 0x1000_0000;
const RCL_GPU: u32 = 0x1100_0000;
const IND_GPU: u32 = 0x1200_0000;
const LIST_BYTES: usize = 16 * 1024;

/// What the job builder wrote for one job with no draws.
struct Lists {
    bcl: Vec<Dec>,
    rcl: Vec<Dec>,
    tile: Vec<Dec>,
}

impl Lists {
    fn rcl_named(&self, name: &str) -> Vec<&Dec> {
        self.rcl.iter().filter(|d| d.name == name).collect()
    }
    fn tile_named(&self, name: &str) -> Vec<&Dec> {
        self.tile.iter().filter(|d| d.name == name).collect()
    }
}

fn build(t: &V3dJobTarget, c: &V3dJobClear) -> Option<Lists> {
    let mut bcl_buf = vec![0u8; LIST_BYTES];
    let mut rcl_buf = vec![0u8; LIST_BYTES];
    let mut ind_buf = vec![0u8; LIST_BYTES];
    let mut bcl = V3dCl {
        buf: bcl_buf.as_mut_ptr(),
        gpu: BCL_GPU,
        size: LIST_BYTES as u32,
        used: 0,
        overflow: false,
        bad_field: false,
    };
    let mut rcl = V3dCl {
        buf: rcl_buf.as_mut_ptr(),
        gpu: RCL_GPU,
        size: LIST_BYTES as u32,
        used: 0,
        overflow: false,
        bad_field: false,
    };
    let mut ind = V3dCl {
        buf: ind_buf.as_mut_ptr(),
        gpu: IND_GPU,
        size: LIST_BYTES as u32,
        used: 0,
        overflow: false,
        bad_field: false,
    };
    let mut job = V3dJob::ZERO;
    let (mut start, mut end) = (0u32, 0u32);
    // SAFETY: the C signatures of v3d_job.h; the lists are live for the calls.
    let ok = unsafe {
        let begin: BeginFn = std::mem::transmute(sym("v3d_job_begin"));
        let finish: EndFn = std::mem::transmute(sym("v3d_job_end"));
        begin(&mut job, t, &mut bcl, &mut rcl, &mut ind) && finish(&mut job, c, &mut start, &mut end)
    };
    if !ok {
        return None;
    }
    let tile_bytes = &ind_buf[(start - IND_GPU) as usize..(end - IND_GPU) as usize];
    Some(Lists {
        bcl: decode(&bcl_buf[..bcl.used as usize]),
        rcl: decode(&rcl_buf[..rcl.used as usize]),
        tile: decode(tile_bytes),
    })
}

/* ---- the cases ---- */

/// `V3D_OUTPUT_IMAGE_FORMAT_RGBA32F` (v3d_packet.xml:236 "rgba32f" = 9) and
/// `RG32F` (:237 "rg32f" = 10); src/v3d/ffi.rs has no constant for them.
const FMT_RGBA32F: u32 = 9;
const FMT_RG32F: u32 = 10;

/// A format on RT 3 and what Mesa makes of it: type and bpp from
/// v3dx_format_table.c, the clamp from v3dx_rcl.c:396-407 (INT for a pure
/// integer format, else none; v3d_packet.xml:147-152 for the values).
#[derive(Clone, Copy)]
struct Fmt {
    name: &'static str,
    image: u32,
    ty: u32,
    bpp: u32,
    clamp: u32,
    /// Where in v3dx_format_table.c the row is.
    mesa: &'static str,
}

const RGBA8: Fmt = Fmt {
    name: "RGBA8",
    image: V3D_OUTPUT_IMAGE_FORMAT_RGBA8,
    ty: V3D_INTERNAL_TYPE_8,
    bpp: V3D_INTERNAL_BPP_32,
    clamp: V3D_RENDER_TARGET_CLAMP_NONE,
    mesa: "v3dx_format_table.c:225-233",
};
const RGBA16F: Fmt = Fmt {
    name: "RGBA16F",
    image: V3D_OUTPUT_IMAGE_FORMAT_RGBA16F,
    ty: V3D_INTERNAL_TYPE_16F,
    bpp: V3D_INTERNAL_BPP_64,
    clamp: V3D_RENDER_TARGET_CLAMP_NONE,
    mesa: "v3dx_format_table.c:254-261",
};
const RGBA32UI: Fmt = Fmt {
    name: "RGBA32UI",
    image: V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI,
    ty: V3D_INTERNAL_TYPE_32UI,
    bpp: V3D_INTERNAL_BPP_128,
    clamp: V3D_RENDER_TARGET_CLAMP_INT,
    mesa: "v3dx_format_table.c:306-309",
};
const RG32F: Fmt = Fmt {
    name: "RG32F",
    image: FMT_RG32F,
    ty: V3D_INTERNAL_TYPE_32F,
    bpp: V3D_INTERNAL_BPP_64,
    clamp: V3D_RENDER_TARGET_CLAMP_NONE,
    mesa: "v3dx_format_table.c:323-326",
};
const RGBA32F: Fmt = Fmt {
    name: "RGBA32F",
    image: FMT_RGBA32F,
    ty: V3D_INTERNAL_TYPE_32F,
    bpp: V3D_INTERNAL_BPP_128,
    clamp: V3D_RENDER_TARGET_CLAMP_NONE,
    mesa: "v3dx_format_table.c:319-322",
};

const W: u32 = 64;
const H: u32 = 64;
const ADDR: [u32; 4] = [0x2000_0000, 0x2100_0000, 0x2200_0000, 0x2300_0000];
const STRIDE: [u32; 4] = [W * 4, W * 4, W * 4, 0x400];

/// A typed job of four RTs: RT 0-2 RGBA8, RT 3 `f3`.
fn target(f3: u32) -> V3dJobTarget {
    let mut t = V3dJobTarget {
        width: W,
        height: H,
        address: ADDR[0],
        stride: STRIDE[0],
        tile_alloc: 0x3000_0000,
        rts: 4,
        typed: true,
        rt_format: [V3D_OUTPUT_IMAGE_FORMAT_RGBA8, V3D_OUTPUT_IMAGE_FORMAT_RGBA8, V3D_OUTPUT_IMAGE_FORMAT_RGBA8, f3],
        ..Default::default()
    };
    for n in 1..4 {
        t.rt[n - 1] = V3dJobRt { address: ADDR[n], stride: STRIDE[n], ..Default::default() };
    }
    t
}

/// RT 3's clear words: four distinct words, so a word dropped or moved shows.
const CLEAR3: [u32; 4] = [0x1122_3344, 0x5566_7788, 0x99aa_bbcc, 0xddee_ff01];

/// What the tile buffer holds a cleared RT 3 as (v3d_tlb_clear, v3dx_draw.c
/// :1705-1760), per internal bpp: word 0 for 32 bpp, two words at 64, four at 128.
fn clear3_words(bpp: u32) -> [u32; 4] {
    let n = 1 + bpp as usize;
    let n = if n == 3 { 4 } else { n };
    let mut w = [0u32; 4];
    w[..n].copy_from_slice(&CLEAR3[..n]);
    w
}

/// Every RT cleared, RT 3 to `words`.
fn clear_all(words: [u32; 4]) -> V3dJobClear {
    let mut c = V3dJobClear { z: 1.0, ..Default::default() };
    c.clear_rt[0] = [0xa0a1_a2a3, 0, 0, 0];
    c.clear_rt[1] = [0xb0b1_b2b3, 0, 0, 0];
    c.clear_rt[2] = [0xc0c1_c2c3, 0, 0, 0];
    c.clear_rt[3] = words;
    c
}

/// The job builder's lists for `f` on RT 3; a refusal fails the test with its name.
fn build_fmt(f: Fmt, c: &V3dJobClear) -> Lists {
    build(&target(f.image), c).unwrap_or_else(|| {
        panic!("v3d_job_begin/end refuse a four-RT job with {} on RT 3 (output image format {})", f.name, f.image)
    })
}

/* ---- 1. the tile config: COLOR, Common and the binning packet ---- */

fn check_color_cfg(f: Fmt) {
    let l = build_fmt(f, &clear_all(clear3_words(f.bpp)));
    let rows = l.rcl_named("Tile Rendering Mode Cfg (Color)");
    assert_eq!(rows.len(), 1, "one TILE_RENDERING_MODE_CFG_COLOR (v3d_packet.xml:1062)");
    let c = rows[0];
    /* RT 3: v3d_packet.xml:1066-1068 */
    assert_eq!(
        c.get("Render Target 3 Internal BPP"),
        f.bpp as u64,
        "RT 3 internal bpp for {} ({}; v3d_packet.xml:1068)",
        f.name,
        f.mesa
    );
    assert_eq!(
        c.get("Render Target 3 Internal Type"),
        f.ty as u64,
        "RT 3 internal type for {} ({}; v3d_packet.xml:1067)",
        f.name,
        f.mesa
    );
    assert_eq!(
        c.get("Render Target 3 Clamp"),
        f.clamp as u64,
        "RT 3 clamp for {} (v3dx_rcl.c:396-407; v3d_packet.xml:1066)",
        f.name
    );
    /* RT 0-2 stay RGBA8: v3d_packet.xml:1070-1080, v3dx_format_table.c:225 */
    for n in 0..3 {
        assert_eq!(c.get(&format!("Render Target {n} Internal BPP")), 0, "RT {n} bpp (v3d_packet.xml:1072/1076/1080)");
        assert_eq!(c.get(&format!("Render Target {n} Internal Type")), V3D_INTERNAL_TYPE_8 as u64, "RT {n} type");
        assert_eq!(c.get(&format!("Render Target {n} Clamp")), 0, "RT {n} clamp");
    }
    /* the count and the maximum bpp, which RT 3 alone may set (v3dx_draw.c
     * start_binning and v3dx_rcl.c emit_rcl carry job->internal_bpp's maximum) */
    let bin = &l.bcl[0];
    assert_eq!(
        bin.name, "Tile Binning Mode Cfg",
        "the binning list starts with the tile binning mode (v3d_packet.xml:943)"
    );
    assert_eq!(bin.get("Number of Render Targets"), 3, "four RTs, minus one (v3d_packet.xml:953)");
    assert_eq!(
        bin.get("Maximum BPP of all render targets"),
        f.bpp as u64,
        "binning max bpp is RT 3's (v3d_packet.xml:951)"
    );
    let common = l.rcl_named("Tile Rendering Mode Cfg (Common)");
    assert_eq!(common.len(), 1, "one Common packet (v3d_packet.xml:996)");
    assert_eq!(common[0].get("Number of Render Targets"), 3, "four RTs, minus one (v3d_packet.xml:1020)");
    assert_eq!(
        common[0].get("Maximum BPP of all render targets"),
        f.bpp as u64,
        "Common max bpp is RT 3's (v3d_packet.xml:1012)"
    );
}

/* ---- 2. the clear colour of RT 3 ---- */

/// The clear colour parts of RT 3 for a clear of `words`, split as v3dx_rcl.c
/// :833-860 splits the 128-bit value: part 1 bits 0-55, part 2 bits 56-111 (64
/// bpp and up), part 3 bits 112-127 (128 bpp), all of the RT's tile buffer value.
fn check_clear(f: Fmt, words: [u32; 4], what: &str) {
    let l = build_fmt(f, &clear_all(words));
    let v: u128 = words.iter().enumerate().map(|(i, w)| (*w as u128) << (32 * i)).sum();
    let bits = |lo: u32, n: u32| ((v >> lo) & ((1u128 << n) - 1)) as u64;
    let part = |n: u32| -> Vec<&Dec> {
        l.rcl_named(&format!("Tile Rendering Mode Cfg (Clear Colors Part{n})"))
            .into_iter()
            .filter(|d| d.get("Render Target number") == 3)
            .collect()
    };
    let (p1, p2, p3) = (part(1), part(2), part(3));
    assert_eq!(p1.len(), 1, "{what}: one CLEAR_COLORS_PART1 for RT 3 (v3d_packet.xml:1103)");
    assert_eq!(
        p1[0].get("Clear Color low 32 bits"),
        bits(0, 32),
        "{what}: part 1 low 32 bits (v3dx_rcl.c:833-860, v3d_packet.xml:1106)"
    );
    assert_eq!(
        p1[0].get("Clear Color next 24 bits"),
        bits(32, 24),
        "{what}: part 1 next 24 bits (v3d_packet.xml:1105)"
    );
    assert_eq!(
        p2.len(),
        usize::from(f.bpp >= V3D_INTERNAL_BPP_64),
        "{what}: CLEAR_COLORS_PART2 for RT 3 exactly from 64 bpp ({}; v3d_packet.xml:1125)",
        f.mesa
    );
    if let Some(p) = p2.first() {
        assert_eq!(p.get("Clear Color mid-low 32 bits"), bits(56, 32), "{what}: part 2 mid-low (v3d_packet.xml:1128)");
        assert_eq!(
            p.get("Clear Color mid-high 24 bits"),
            bits(88, 24),
            "{what}: part 2 mid-high (v3d_packet.xml:1127)"
        );
    }
    assert_eq!(
        p3.len(),
        usize::from(f.bpp >= V3D_INTERNAL_BPP_128),
        "{what}: CLEAR_COLORS_PART3 for RT 3 exactly at 128 bpp ({}; v3d_packet.xml:1141)",
        f.mesa
    );
    if let Some(p) = p3.first() {
        assert_eq!(p.get("Clear Color high 16 bits"), bits(112, 16), "{what}: part 3 high 16 (v3d_packet.xml:1145)");
    }
    /* the order within RT 3: part 1, then 2, then 3 (v3dx_rcl.c:833-860) */
    let at = |n: u32| {
        l.rcl.iter().position(|d| {
            d.name == format!("Tile Rendering Mode Cfg (Clear Colors Part{n})").as_str()
                && d.get("Render Target number") == 3
        })
    };
    if f.bpp >= V3D_INTERNAL_BPP_64 {
        assert!(at(1) < at(2), "{what}: part 1 before part 2");
    }
    if f.bpp >= V3D_INTERNAL_BPP_128 {
        assert!(at(2) < at(3), "{what}: part 2 before part 3");
    }
    /* RT 3's parts come after RT 2's, and every part before the COLOR packet
     * (v3dx_rcl.c:833-860 then :898-913) */
    let last_rt2 = l.rcl.iter().rposition(|d| {
        d.name.starts_with("Tile Rendering Mode Cfg (Clear Colors") && d.get("Render Target number") == 2
    });
    assert!(last_rt2 < at(1), "{what}: RT 2's clear colour before RT 3's");
    let color = l.rcl.iter().position(|d| d.name == "Tile Rendering Mode Cfg (Color)").unwrap();
    assert!(at(1).unwrap() < color, "{what}: the clear colours before the COLOR packet");
}

fn check_clear_cleared(f: Fmt) {
    check_clear(f, clear3_words(f.bpp), "cleared RT 3");
}

/* ---- 3. RT 3's load and store in the tile list ---- */

fn loads(l: &Lists) -> Vec<&Dec> {
    l.tile_named("Load Tile Buffer General")
}
fn stores(l: &Lists) -> Vec<&Dec> {
    l.tile_named("Store Tile Buffer General")
}

fn check_load_store(f: Fmt) {
    /* RT 3 not cleared (every RT loaded), as a draw after glClear of another buffer only */
    let mut c = clear_all(clear3_words(f.bpp));
    c.load_rt0 = true;
    c.load_rt = [true, true, true];
    let l = build_fmt(f, &c);
    let ld: Vec<&Dec> = loads(&l).into_iter().filter(|d| d.get("Buffer to Load") == 3).collect();
    assert_eq!(ld.len(), 1, "{}: one load of RT 3 when it is not cleared (v3dx_rcl.c:198-212)", f.name);
    let d = ld[0];
    assert_eq!(d.get("Memory Format"), 0, "{}: RT 3 loads raster (v3d_packet.xml:108, :544)", f.name);
    assert_eq!(
        d.get("Input Image Format"),
        f.image as u64,
        "{}: RT 3 loads in its own format (v3d_packet.xml:545)",
        f.name
    );
    assert_eq!(d.get("Height in UB or Stride"), STRIDE[3] as u64, "{}: the raster stride (v3d_packet.xml:540)", f.name);
    assert_eq!(d.get("Address"), ADDR[3] as u64, "{}: RT 3's address (v3d_packet.xml:531)", f.name);
    assert_eq!(d.get("Decimate mode"), 0, "{}: sample 0 (v3d_packet.xml:117)", f.name);
    assert_eq!(d.get("R/B swap"), 0, "{}: no R/B swap on RT 3", f.name);
    assert_eq!(d.get("Force alpha 1"), 0, "{}: no forced alpha on RT 3", f.name);
    /* all four loads, in RT order, before END_OF_LOADS (v3dx_rcl.c:198-212) */
    let order: Vec<u64> = loads(&l).iter().map(|d| d.get("Buffer to Load")).collect();
    assert_eq!(order, [0, 1, 2, 3], "{}: loads in RT order", f.name);
    let eol = l.tile.iter().position(|d| d.name == "End of Loads").unwrap();
    let last_load = l.tile.iter().rposition(|d| d.name == "Load Tile Buffer General").unwrap();
    assert!(last_load < eol, "{}: RT 3's load is before END_OF_LOADS", f.name);
    /* its clear colour is not read: the parts are there, zero (v3dx_rcl.c:833-860) */
    check_clear_loaded(f, &l);
    check_store(f, &l);
    assert_eq!(c.load_rt.len(), 3);

    /* RT 3 cleared: no load of it (a cleared RT is not loaded) */
    let mut c = clear_all(clear3_words(f.bpp));
    c.load_rt0 = true;
    c.load_rt = [true, true, false];
    let l = build_fmt(f, &c);
    let order: Vec<u64> = loads(&l).iter().map(|d| d.get("Buffer to Load")).collect();
    assert_eq!(order, [0, 1, 2], "{}: a cleared RT 3 is not loaded", f.name);
    check_store(f, &l);
}

fn check_clear_loaded(f: Fmt, l: &Lists) {
    let colour_fields: [(&str, &[&str]); 3] = [
        ("Part1", &["Clear Color low 32 bits", "Clear Color next 24 bits"]),
        ("Part2", &["Clear Color mid-low 32 bits", "Clear Color mid-high 24 bits"]),
        ("Part3", &["Clear Color high 16 bits"]),
    ];
    for (part, fields) in colour_fields {
        for d in l
            .rcl_named(&format!("Tile Rendering Mode Cfg (Clear Colors {part})"))
            .into_iter()
            .filter(|d| d.get("Render Target number") == 3)
        {
            for field in fields {
                assert_eq!(d.get(field), 0, "{}: loaded RT 3's {part} {field} is zero", f.name);
            }
        }
    }
}

fn check_store(f: Fmt, l: &Lists) {
    let st = stores(l);
    let bufs: Vec<u64> = st.iter().map(|d| d.get("Buffer to Store")).collect();
    assert_eq!(
        bufs,
        [0, 1, 2, 3],
        "{}: one store per RT in RT order at the end of the tile (v3dx_rcl.c:258-283, v3d_packet.xml:512)",
        f.name
    );
    let d = st[3];
    assert_eq!(d.get("Memory Format"), 0, "{}: RT 3 stores raster (v3d_packet.xml:108, :511)", f.name);
    assert_eq!(
        d.get("Output Image Format"),
        f.image as u64,
        "{}: RT 3 stores in its own format (v3d_packet.xml:505)",
        f.name
    );
    assert_eq!(d.get("Height in UB or Stride"), STRIDE[3] as u64, "{}: the raster stride (v3d_packet.xml:497)", f.name);
    assert_eq!(d.get("Address"), ADDR[3] as u64, "{}: RT 3's address (v3d_packet.xml:492)", f.name);
    assert_eq!(d.get("Decimate mode"), 0, "{}: sample 0 (v3d_packet.xml:507)", f.name);
    assert_eq!(d.get("R/B swap"), 0, "{}: no R/B swap on RT 3", f.name);
    /* the stores are after the branch to the implicit tile list and before the
     * tile's end (v3d_rcl_emit_generic_per_tile_list) */
    let at = |n: &str| l.tile.iter().position(|d| d.name == n).unwrap_or_else(|| panic!("{n} in the tile list"));
    let branch = at("Branch to Implicit Tile List");
    let first = l.tile.iter().position(|d| d.name == "Store Tile Buffer General").unwrap();
    let last = l.tile.iter().rposition(|d| d.name == "Store Tile Buffer General").unwrap();
    assert!(
        branch < first && last < at("End of Tile Marker"),
        "{}: stores between the branch and END_OF_TILE_MARKER",
        f.name
    );
    assert_eq!(l.tile.last().unwrap().name, "Return from sub-list", "{}: the tile list returns", f.name);
}

/* One test per format for each of the three groups. */
macro_rules! rt3_tests {
    ($($(#[$m:meta])* $fmt:ident: $cfg:ident, $clear:ident, $loadstore:ident;)*) => {$(
        $(#[$m])* #[test] fn $cfg() { check_color_cfg($fmt); }
        $(#[$m])* #[test] fn $clear() { check_clear_cleared($fmt); }
        $(#[$m])* #[test] fn $loadstore() { check_load_store($fmt); }
    )*};
}

rt3_tests! {
    RGBA8: rt3_cfg_rgba8, rt3_clear_rgba8, rt3_load_store_rgba8;
    RGBA16F: rt3_cfg_rgba16f, rt3_clear_rgba16f, rt3_load_store_rgba16f;
    RG32F: rt3_cfg_rg32f, rt3_clear_rg32f, rt3_load_store_rg32f;
    RGBA32UI: rt3_cfg_rgba32ui, rt3_clear_rgba32ui, rt3_load_store_rgba32ui;
    RGBA32F: rt3_cfg_rgba32f, rt3_clear_rgba32f, rt3_load_store_rgba32f;
}

/* ---- 4. the rest of RT 3's cases ---- */

/// RT 3 holding more than RGBA8 sets the tile's maximum bpp alone; RT 0 at 128
/// bpp with RT 3 at 32 sets it from RT 0 (the maximum is over all RTs).
#[test]
fn max_bpp_is_over_all_rts() {
    let mut t = target(V3D_OUTPUT_IMAGE_FORMAT_RGBA8);
    t.rt_format[0] = V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI;
    let l = build(&t, &clear_all([1, 2, 3, 4])).expect("RT 0 RGBA32UI, RT 3 RGBA8");
    let c = l.rcl_named("Tile Rendering Mode Cfg (Color)")[0];
    assert_eq!(c.get("Render Target 0 Internal BPP"), 2, "RT 0 128 bpp (v3dx_format_table.c:306)");
    assert_eq!(c.get("Render Target 3 Internal BPP"), 0, "RT 3 stays 32 bpp (v3dx_format_table.c:225)");
    assert_eq!(l.bcl[0].get("Maximum BPP of all render targets"), 2, "maximum over the RTs (v3d_packet.xml:951)");
    assert_eq!(l.rcl_named("Tile Rendering Mode Cfg (Common)")[0].get("Maximum BPP of all render targets"), 2);
    /* RT 3 at 32 bpp still gets its one clear colour packet and no part 2/3 */
    let rt3: Vec<&Dec> = l
        .rcl
        .iter()
        .filter(|d| d.name.starts_with("Tile Rendering Mode Cfg (Clear Colors") && d.get("Render Target number") == 3)
        .collect();
    assert_eq!(rt3.len(), 1, "RT 3 at 32 bpp: part 1 only (v3dx_rcl.c:833-860)");
}

/// The untyped job (every ES 2.0 multiple-render-target job): RT 3 is RGBA8
/// raster, its clear colour the one word `rgba8_rt[2]`.
#[test]
fn untyped_rt3_is_rgba8() {
    let mut t = target(0);
    t.typed = false;
    t.rt_format = [0; 4];
    let mut c = V3dJobClear { z: 1.0, ..Default::default() };
    c.rgba8 = 0x0102_0304;
    c.rgba8_rt = [0x1112_1314, 0x2122_2324, 0x3132_3334];
    let l = build(&t, &c).expect("an untyped four-RT job");
    let col = l.rcl_named("Tile Rendering Mode Cfg (Color)")[0];
    assert_eq!(col.get("Render Target 3 Internal BPP"), 0);
    assert_eq!(
        col.get("Render Target 3 Internal Type"),
        V3D_INTERNAL_TYPE_8 as u64,
        "RGBA8 (v3dx_format_table.c:225-233)"
    );
    assert_eq!(col.get("Render Target 3 Clamp"), 0);
    let p1: Vec<&Dec> = l
        .rcl_named("Tile Rendering Mode Cfg (Clear Colors Part1)")
        .into_iter()
        .filter(|d| d.get("Render Target number") == 3)
        .collect();
    assert_eq!(p1.len(), 1, "one part 1 for RT 3");
    assert_eq!(p1[0].get("Clear Color low 32 bits"), 0x3132_3334, "RT 3's word is rgba8_rt[2] (v3dx_rcl.c:833-860)");
    assert_eq!(p1[0].get("Clear Color next 24 bits"), 0);
    assert!(l
        .rcl_named("Tile Rendering Mode Cfg (Clear Colors Part2)")
        .iter()
        .all(|d| d.get("Render Target number") != 3));
    let st = stores(&l);
    assert_eq!(
        st[3].get("Output Image Format"),
        V3D_OUTPUT_IMAGE_FORMAT_RGBA8 as u64,
        "RGBA8 store (v3d_packet.xml:505)"
    );
    assert_eq!(st[3].get("Height in UB or Stride"), STRIDE[3] as u64);
}

/// A GL_NONE fourth draw buffer (address 0): configured and cleared in the tile
/// buffer so a program's write has room, never loaded or stored (v3dx_rcl.c:
/// 198-212 and :258-283 skip an RT with no surface).
#[test]
fn rt3_without_buffer_is_configured_not_loaded_or_stored() {
    let mut t = target(V3D_OUTPUT_IMAGE_FORMAT_RGBA16F);
    t.rt[2].address = 0;
    let mut c = clear_all(clear3_words(V3D_INTERNAL_BPP_64));
    c.load_rt0 = true;
    c.load_rt = [true, true, true];
    let l = build(&t, &c).expect("a four-RT job with RT 3 GL_NONE");
    let col = l.rcl_named("Tile Rendering Mode Cfg (Color)")[0];
    assert_eq!(col.get("Render Target 3 Internal BPP"), 1, "still configured as RGBA16F (v3d_packet.xml:1068)");
    assert_eq!(col.get("Render Target 3 Internal Type"), V3D_INTERNAL_TYPE_16F as u64);
    let lb: Vec<u64> = loads(&l).iter().map(|d| d.get("Buffer to Load")).collect();
    let sb: Vec<u64> = stores(&l).iter().map(|d| d.get("Buffer to Store")).collect();
    assert_eq!(lb, [0, 1, 2], "RT 3 is not loaded");
    assert_eq!(sb, [0, 1, 2], "RT 3 is not stored");
}

/// RT 3 of each format is stored in a job with RT 0-2 stored: the store of RT 3
/// follows RT 2's and no store is added (4 stores for 4 RTs, none of buffer
/// NONE: v3dx_rcl.c:315-321 stores NONE only when nothing else is stored).
#[test]
fn four_stores_and_no_none_store() {
    let l = build(&target(V3D_OUTPUT_IMAGE_FORMAT_RGBA16F), &clear_all([0; 4])).unwrap();
    let sb: Vec<u64> = stores(&l).iter().map(|d| d.get("Buffer to Store")).collect();
    assert_eq!(sb, [0, 1, 2, 3], "exactly the four RT stores (v3d_packet.xml:512, :521 \"None\" = 8)");
    let clear = l.tile_named("Clear Tile Buffers");
    assert_eq!(clear.len(), 1, "CLEAR_TILE_BUFFERS once, after the stores (v3dx_rcl.c:330-337)");
    assert_eq!(clear[0].get("Clear all Render Targets"), 1, "all RTs, RT 3 included (v3d_packet.xml:482)");
}

/* ---- 5. a draw's blend state reaches RT 3 ---- */

type DrawFn = unsafe extern "C" fn(*mut V3dJob, *const V3dJobDraw, *mut c_void) -> i32;

/// The binning list of a four-RGBA8-RT job after one triangle drawn with `st`.
fn draw_bcl(st: &V3dJobState) -> Vec<Dec> {
    let mut bcl_buf = vec![0u8; LIST_BYTES];
    let mut rcl_buf = vec![0u8; LIST_BYTES];
    let mut ind_buf = vec![0u8; LIST_BYTES];
    let cl = |buf: &mut Vec<u8>, gpu: u32| V3dCl {
        buf: buf.as_mut_ptr(),
        gpu,
        size: LIST_BYTES as u32,
        used: 0,
        overflow: false,
        bad_field: false,
    };
    let (mut bcl, mut rcl, mut ind) = (cl(&mut bcl_buf, BCL_GPU), cl(&mut rcl_buf, RCL_GPU), cl(&mut ind_buf, IND_GPU));
    let fs = V3dFsProg {
        code: 0x0020_0000,
        threads: 4,
        single_seg: false,
        num_inputs: 0,
        flat_shade_flags: 0,
        noperspective_flags: 0,
        centroid_flags: 0,
        flat_shade_flags_hi: 0,
        noperspective_flags_hi: 0,
        centroid_flags_hi: 0,
        writes_z: false,
        disable_ez: false,
        uses_center_w: false,
        lock_scoreboard_on_first_thrsw: false,
        uses_implicit_point_line_varyings: false,
        uses_pid: false,
        uniforms: core::ptr::null(),
        num_uniforms: 0,
    };
    let vs_prog = |code: u32| V3dVsProg {
        code,
        threads: 4,
        single_seg: true,
        vpm_input_size: 0,
        vpm_output_size: 2,
        separate_segments: false,
        vcm_cache_size: 4,
        uses_vid: false,
        uses_iid: false,
        uses_biid: false,
        vattr_sizes: [0; 16],
        uniforms: core::ptr::null(),
        num_uniforms: 0,
    };
    let (vs, cs) = (vs_prog(0x0020_1000), vs_prog(0x0020_2000));
    let d = V3dJobDraw {
        st,
        fs: &fs,
        vs: &vs,
        cs: &cs,
        vs_storage: V3dUniformStorage::NONE,
        fs_storage: V3dUniformStorage::NONE,
        attrs: core::ptr::null(),
        num_attrs: 0,
        prim: V3D_PRIM_TRIANGLES,
        count: 3,
        first: 0,
        indexed: false,
        index_type: 0,
        index_buffer: 0,
        index_buffer_bytes: 0,
        index_offset: 0,
        tmu: core::ptr::null(),
        num_tmu: 0,
        vs_tmu: core::ptr::null(),
        num_vs_tmu: 0,
        vs_writes_psiz: false,
        instances: 0,
    };
    let t = target(V3D_OUTPUT_IMAGE_FORMAT_RGBA8);
    let mut job = V3dJob::ZERO;
    // SAFETY: the C signatures of v3d_job.h; the lists, the programs and the
    // state are live for the calls.
    let r = unsafe {
        let begin: BeginFn = std::mem::transmute(sym("v3d_job_begin"));
        let draw: DrawFn = std::mem::transmute(sym("v3d_job_draw"));
        assert!(begin(&mut job, &t, &mut bcl, &mut rcl, &mut ind), "a four-RT job begins");
        draw(&mut job, &d, core::ptr::null_mut())
    };
    assert_eq!(r, V3D_JOB_OK, "the draw goes in");
    decode(&bcl_buf[..bcl.used as usize])
}

/// The draw state: a 64x64 viewport, colour mask RGBA, blending as `blend`.
fn blend_state(blend: bool) -> V3dJobState {
    V3dJobState {
        vp: V3dViewport { scale: [32.0, -32.0, 0.5], translate: [32.0, 32.0, 0.5] },
        line_width: 1.0,
        blend_enabled: blend,
        rgb_src: V3D_BLEND_FACTOR_SRC_ALPHA,
        rgb_dst: V3D_BLEND_FACTOR_INV_SRC_ALPHA,
        alpha_src: V3D_BLEND_FACTOR_ONE,
        alpha_dst: V3D_BLEND_FACTOR_INV_SRC_ALPHA,
        colormask: 0xf,
        ..Default::default()
    }
}

/// glEnable(GL_BLEND) blends every draw buffer, RT 3 included: without
/// independent blend Mesa's blend_enables is (1 << max_rts) - 1, max_rts 4
/// (v3dx_state.c:148-156), emitted as BLEND_ENABLES's mask (v3dx_emit.c:529-531),
/// and the one BLEND_CFG names every RT (:559-562). v3d_job.c wrote mask 1 (RT 0
/// alone) before v3d-frame4.
#[test]
fn blend_enables_names_rt3() {
    let bcl = draw_bcl(&blend_state(true));
    let en: Vec<&Dec> = bcl.iter().filter(|d| d.name == "Blend Enables").collect();
    assert_eq!(en.len(), 1, "one BLEND_ENABLES (v3d_packet.xml:773)");
    assert_eq!(en[0].get("Mask"), 0xf, "RT 0-3 blend (v3dx_state.c:154-155)");
    let cfg: Vec<&Dec> = bcl.iter().filter(|d| d.name == "Blend Cfg").collect();
    assert_eq!(cfg.len(), 1, "one BLEND_CFG without independent blend (v3dx_emit.c:559-562)");
    assert_eq!(cfg[0].get("Render Target Mask"), 0xf, "BLEND_CFG for RT 0-3 (v3d_packet.xml:778)");
    assert_eq!(cfg[0].get("Color blend src factor"), V3D_BLEND_FACTOR_SRC_ALPHA as u64);
    assert_eq!(cfg[0].get("Color blend dst factor"), V3D_BLEND_FACTOR_INV_SRC_ALPHA as u64);
    assert_eq!(cfg[0].get("Alpha blend src factor"), V3D_BLEND_FACTOR_ONE as u64);
}

/// Without blending, BLEND_ENABLES is 0 and there is no BLEND_CFG
/// (v3dx_emit.c:564-567).
#[test]
fn blend_off_names_no_rt() {
    let bcl = draw_bcl(&blend_state(false));
    let en: Vec<&Dec> = bcl.iter().filter(|d| d.name == "Blend Enables").collect();
    assert_eq!(en.len(), 1, "one BLEND_ENABLES (v3d_packet.xml:773)");
    assert_eq!(en[0].get("Mask"), 0, "no RT blends");
    assert!(bcl.iter().all(|d| d.name != "Blend Cfg"), "no BLEND_CFG");
}

/// Per-RT blend enables (`rt_blend_own`, item v3d-blend-rt): BLEND_ENABLES is
/// the job's mask, Mesa's blend_enables of an independent blend state
/// (v3dx_state.c:148-156, v3dx_emit.c:525-531): an integer RT 3 left out
/// (0b0111). BLEND_CFG stays the one state over every RT.
#[test]
fn blend_enables_follow_the_per_rt_mask() {
    let st = V3dJobState { rt_blend_own: true, rt_blend_mask: 0b0111, ..blend_state(true) };
    let bcl = draw_bcl(&st);
    let en: Vec<&Dec> = bcl.iter().filter(|d| d.name == "Blend Enables").collect();
    assert_eq!(en.len(), 1, "one BLEND_ENABLES (v3d_packet.xml:773)");
    assert_eq!(en[0].get("Mask"), 0b0111, "RT 3 does not blend");
    let cfg: Vec<&Dec> = bcl.iter().filter(|d| d.name == "Blend Cfg").collect();
    assert_eq!(cfg.len(), 1);
    assert_eq!(cfg[0].get("Render Target Mask"), 0xf);
    /* the mask means nothing without blending */
    let st = V3dJobState { rt_blend_own: true, rt_blend_mask: 0b0111, ..blend_state(false) };
    let bcl = draw_bcl(&st);
    let en: Vec<&Dec> = bcl.iter().filter(|d| d.name == "Blend Enables").collect();
    assert_eq!(en[0].get("Mask"), 0);
}

/// A GL_NONE draw buffer 3 (`rt_colormask` nibble 0 for RT 3): COLOR_WRITE_MASKS
/// disables every channel of RT 3 (bits 12-15) and keeps RT 0-2 written
/// (v3dx_emit.c:571-585).
#[test]
fn none_rt3_write_mask_is_full() {
    let st = V3dJobState { rt_colormask_own: true, rt_colormask: 0x0ff, ..blend_state(false) };
    let bcl = draw_bcl(&st);
    let wm: Vec<&Dec> = bcl.iter().filter(|d| d.name == "Color Write Masks").collect();
    assert_eq!(wm.len(), 1);
    assert_eq!(wm[0].get("Mask"), 0xf000, "RT 3 masked, RT 0-2 written");
}
