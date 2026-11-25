// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The fixed-function packets of the V3D 7.1 C job builder
//! (driver/src/v3d/v3d_job71.c over v3d_frame.c at `-DV3D_CL_VER=71`, the
//! builder the Pi 5 runs) against Mesa's 7.1 references refs71b added:
//! tools/v3d/pi5/gl_blendsub.clif, gl_depthfuncs.clif and gl_primswide.clif.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_job71_fixed
//! ```
//!
//! tests/v3d71_compose5.rs names these packets as `KNOWN_WORD_GAPS` of the
//! Rust modules (no `PacketSpec` in draw71), so until now nothing compared
//! the values the C job writes for them with Mesa's. Compared here:
//!
//! * in the binning list, at every draw, the state in effect (the last
//!   packet of each kind before the draw packet, tools/v3d/check71.d
//!   gl_cmp.py's `bcl_split`): `POINT_SIZE`, `LINE_WIDTH`,
//!   `OCCLUSION_QUERY_COUNTER` (CLIF `OCCLUSION_QUERY_COUNTER_ENABLE`),
//!   `SAMPLE_STATE`, `VCM_CACHE_SIZE`, `ZERO_ALL_FLAT_SHADE_FLAGS`,
//!   `ZERO_ALL_NON_PERSPECTIVE_FLAGS`, `ZERO_ALL_CENTROID_FLAGS` (CLIF
//!   `ZERO_ALL_FLATSHADE_FLAGS`, `ZERO_ALL_NOPERSPECTIVE_FLAGS`,
//!   `ZERO_ALL_CENTROID_FLAGS`) and `BLEND_CONSTANT_COLOR` (CLIF
//!   `BLEND_CCOLOR`);
//! * the prologue's `OCCLUSION_QUERY_COUNTER` (v3d_job71_begin's);
//! * `PRIM_LIST_FORMAT` of the generic tile list (v3d_job71_end's, in the
//!   indirect buffer between the tile list's start and end).
//!
//! Every packet is decoded from our bytes with the packet table of
//! third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml at version 71
//! (as tests/v3d71_backend71_shapes.rs decodes them) and compared as the
//! field values the dump prints, not as BO addresses: an address field is
//! `[null]` when 0 and `[addr]` otherwise, on both sides.
//!
//! The C builder runs in a child process: tests/v3d71_backend71_harness.c,
//! unchanged, built and driven as tests/v3d71_backend71_shapes.rs does. Its
//! inputs are each shape's GL state as gallium hands it to the driver (the
//! GL each shape issues is tools/v3d/pi5/src/gl_shapes.py, `DRAW2_SHAPES`):
//! the 64x64 RGBA8 window, the viewport, the line width, depth and blend
//! state, a zero blend colour, no occlusion query. The programs' facts these
//! packets read (the VCM cache sizes and the varying flags) are our 7.1
//! compiler's gl_test pair, tools/v3d/gltest-shaders71/ours_color_uv.h (read
//! as data); the uniform streams are left empty (no packet compared here
//! reads them). gl_primswide's vertex shader is gl_test's with
//! `gl_PointSize = 16.0`, which has no pinned program of ours: the colour
//! pair stands in for it (the VCM cache sizes and the flags are the same
//! for both in Mesa's dump).
//!
//! A value difference must be one entry of [`PINNED`] (with its reason and
//! an `#[ignore]` gap test that fails until the C job writes Mesa's value),
//! and every entry must still occur. Today the list is empty: at every draw
//! of the three shapes the C job's values are Mesa's (2026-11-25, at
//! 888583fc). Mesa emits these packets only when
//! their dirty bit is set; v3d_job71 writes every state packet on every draw
//! (`emit_state` "with every dirty bit set"). That emission difference is
//! not a value difference: [`EMISSIONS`] pins the counts of both sides per
//! shape and packet.
#![allow(dead_code)]

#[path = "../src/v3d/ffi71.rs"]
mod ffi71;

use ffi71::*;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn num(s: &str) -> u64 {
    let s = s.trim().trim_end_matches(['u', 'U', 'l', 'L']);
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).unwrap_or_else(|e| panic!("{s}: {e}")),
        None => s.parse().unwrap_or_else(|e| panic!("{s}: {e}")),
    }
}

/* ---- where the job's buffers sit (tests/v3d71_backend71_shapes.rs's) ---- */

const G_BCL: u32 = 0x100000;
const G_RCL: u32 = 0x110000;
const G_IND: u32 = 0x120000;
const G_CODE: u32 = 0x200000;
const G_VTX: u32 = 0x300000;
const G_TILE: u32 = 0x400000;
const G_RT0: u32 = 0x1000000;
const G_ZS: u32 = 0x2000000;
const CL_SIZE: u32 = 0x4000;
/// The Pi 5 kernel's page (backend71::PAGE_SIZE); only the allocation sizes
/// the harness reports read it.
const PAGE_SIZE: u32 = 16 * 1024;
const W: u32 = 64;
const H: u32 = 64;

/* ---- our gl_test program's facts (ours_color_uv.h, read as data) ---- */

const OURS_COLOR_UV: &str = include_str!("../tools/v3d/gltest-shaders71/ours_color_uv.h");

/// The header's plain `#define NAME number` lines.
fn defines() -> &'static BTreeMap<String, u64> {
    static D: OnceLock<BTreeMap<String, u64>> = OnceLock::new();
    D.get_or_init(|| {
        let mut m = BTreeMap::new();
        for line in OURS_COLOR_UV.lines() {
            let mut it = line.split_whitespace();
            if it.next() != Some("#define") {
                continue;
            }
            let (Some(name), Some(v)) = (it.next(), it.next()) else { continue };
            if v.starts_with(|c: char| c.is_ascii_digit()) {
                m.insert(name.to_string(), num(v));
            }
        }
        m
    })
}

fn d(name: &str) -> u32 {
    let key = format!("OURS71_COLOR_UV_{name}");
    *defines().get(&key).unwrap_or_else(|| panic!("{key} not in ours_color_uv.h")) as u32
}

fn vattr_sizes(stage: &str) -> [u8; 16] {
    let key = format!("OURS71_COLOR_UV_{stage}_VATTR_SIZES");
    let line = OURS_COLOR_UV.lines().find(|l| l.contains(&key)).unwrap_or_else(|| panic!("{key}"));
    let v: Vec<u8> =
        line[line.find('{').unwrap() + 1..line.find('}').unwrap()].split(',').map(|s| num(s) as u8).collect();
    v.try_into().unwrap()
}

fn fs_prog() -> V3dFsProg {
    V3dFsProg {
        code: G_CODE,
        threads: d("FS_THREADS"),
        single_seg: d("FS_SINGLE_SEG") != 0,
        num_inputs: d("FS_NUM_INPUTS"),
        flat_shade_flags: d("FS_FLAT_SHADE_FLAGS"),
        noperspective_flags: d("FS_NOPERSPECTIVE_FLAGS"),
        centroid_flags: d("FS_CENTROID_FLAGS"),
        flat_shade_flags_hi: 0,
        noperspective_flags_hi: 0,
        centroid_flags_hi: 0,
        writes_z: d("FS_WRITES_Z") != 0,
        disable_ez: d("FS_DISABLE_EZ") != 0,
        uses_center_w: d("FS_USES_CENTER_W") != 0,
        lock_scoreboard_on_first_thrsw: d("FS_LOCK_SCOREBOARD_ON_FIRST_THRSW") != 0,
        uses_implicit_point_line_varyings: d("FS_USES_IMPLICIT_POINT_LINE_VARYINGS") != 0,
        uses_pid: d("FS_USES_PID") != 0,
        uniforms: std::ptr::null(),
        num_uniforms: 0,
    }
}

fn vs_prog(stage: &str, code: u32) -> V3dVsProg {
    let g = |n: &str| d(&format!("{stage}_{n}"));
    V3dVsProg {
        code,
        threads: g("THREADS"),
        single_seg: g("SINGLE_SEG") != 0,
        vpm_input_size: g("VPM_INPUT_SIZE"),
        vpm_output_size: g("VPM_OUTPUT_SIZE"),
        separate_segments: g("SEPARATE_SEGMENTS") != 0,
        vcm_cache_size: g("VCM_CACHE_SIZE"),
        uses_vid: g("USES_VID") != 0,
        uses_iid: g("USES_IID") != 0,
        uses_biid: g("USES_BIID") != 0,
        vattr_sizes: vattr_sizes(stage),
        uniforms: std::ptr::null(),
        num_uniforms: 0,
    }
}

/// gl_test's 40-byte vertex: pos4 +0, colour4 +16, uv2 +32.
const ATTRS: [V3dVertexAttr; 3] = [
    V3dVertexAttr { address: G_VTX, stride: 40, components: 4, divisor: 0 },
    V3dVertexAttr { address: G_VTX + 16, stride: 40, components: 4, divisor: 0 },
    V3dVertexAttr { address: G_VTX + 32, stride: 40, components: 2, divisor: 0 },
];

/* ---- the C builder in a child process --------------------------------------- */

const HARNESS_C: &str = include_str!("v3d71_backend71_harness.c");

fn find_cc() -> String {
    for c in [std::env::var("CC").unwrap_or_default(), "cc".to_string()] {
        let Some(prog) = c.split_whitespace().next() else { continue };
        if Command::new(prog).arg("--version").output().is_ok() {
            return c;
        }
    }
    panic!("no C compiler ($CC or cc): this test runs driver/src/v3d/v3d_job71.c");
}

fn harness() -> &'static PathBuf {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let cc = find_cc();
        // One directory per process: the gate may run this binary in several
        // rows at once with the same CARGO_TARGET_TMPDIR.
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("v3d71_job71_fixed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("harness");
        let src = dir.join("harness.c");
        std::fs::write(&src, HARNESS_C).unwrap();
        let mesa = format!("{ROOT}/third_party/mesa-26.2.1");
        let mut parts = cc.split_whitespace();
        let out = Command::new(parts.next().unwrap())
            .args(parts)
            .args(["-O1", "-std=c11", "-Wall", "-Wextra", "-Werror", "-D_GNU_SOURCE", "-DHAVE_ENDIAN_H"])
            .arg("-DV3D_CL_VER=71")
            .args(["-isystem", &format!("{mesa}/src"), "-isystem", &format!("{mesa}/include")])
            .args(["-isystem", &format!("{mesa}/src/broadcom")])
            .arg(format!("-I{ROOT}/driver/src/v3d"))
            .arg(format!("-I{ROOT}/driver/src/v3d/generated"))
            .arg(&src)
            .arg(format!("{ROOT}/driver/src/v3d/v3d_job71.c"))
            .arg(format!("{ROOT}/driver/src/v3d/v3d_frame.c"))
            .arg("-o")
            .arg(&exe)
            .arg("-lm")
            .output()
            .expect("run the C compiler");
        assert!(out.status.success(), "harness build failed:\n{}", String::from_utf8_lossy(&out.stderr));
        exe
    })
}

/// The raw bytes of a `#[repr(C)]` plain struct.
fn raw<T>(v: &T) -> Vec<u8> {
    // SAFETY: a plain repr(C) struct read as bytes; padding is never read by C.
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), std::mem::size_of::<T>()).to_vec() }
}

fn raw_slice<T>(v: &[T]) -> Vec<u8> {
    // SAFETY: as `raw`, for a slice of plain structs.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)).to_vec() }
}

struct Rd<'a>(&'a [u8]);

impl Rd<'_> {
    fn take(&mut self, n: usize) -> &[u8] {
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        a
    }
    fn u32(&mut self) -> u32 {
        u32::from_ne_bytes(self.take(4).try_into().unwrap())
    }
}

/// One draw of a shape: its state, primitive and vertex range.
struct Draw {
    st: V3dJob71State,
    prim: u32,
    count: u32,
    first: u32,
}

/// What the C job wrote: the binning list and the generic tile list.
struct Ours {
    bcl: Vec<u8>,
    tiles: Vec<u8>,
}

/// v3d_job71_begin, v3d_job71_draw for each draw, v3d_job71_end, through the
/// harness's stdin/stdout protocol (its header).
fn build(target: &V3dJob71Target, clear: &V3dJob71Clear, draws: &[Draw]) -> Ours {
    let (fs, vs, cs) = (fs_prog(), vs_prog("VS", G_CODE + 0x100), vs_prog("CS", G_CODE + 0x200));
    let mut inp = vec![];
    let w = |v: &mut Vec<u8>, x: u32| v.extend(x.to_ne_bytes());
    w(&mut inp, 0x3731_4a42);
    w(&mut inp, draws.len() as u32);
    inp.extend(raw(target));
    inp.extend(raw(clear));
    for x in [G_BCL, G_RCL, G_IND, CL_SIZE, PAGE_SIZE] {
        w(&mut inp, x);
    }
    for dr in draws {
        inp.extend(raw(&dr.st));
        inp.extend(raw(&fs));
        inp.extend(raw(&vs));
        inp.extend(raw(&cs));
        for x in [dr.prim, dr.count, dr.first] {
            w(&mut inp, x);
        }
        // no uniform words, no GL uniform storage, three attribute records
        for x in [0, 0, 0, 0, 0, ATTRS.len() as u32] {
            w(&mut inp, x);
        }
        inp.extend(raw_slice(&ATTRS));
    }
    let mut child = Command::new(harness()).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(&inp).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "harness exit {:?}", out.status.code());
    let mut r = Rd(&out.stdout);
    let (status, at, code) = (r.u32(), r.u32(), r.u32() as i32);
    assert_eq!(status, 0, "the C job refused the shape (status {status}, draw {at}, code {code})");
    r.take(std::mem::size_of::<V3dTiling>());
    let n = r.u32();
    r.take(n as usize * 5 * 4);
    let (ls, le, _qms, _qts) = (r.u32(), r.u32(), r.u32(), r.u32());
    let mut lists: Vec<Vec<u8>> = vec![];
    for _ in 0..3 {
        let used = r.u32() as usize;
        lists.push(r.take(used).to_vec());
    }
    let ind = &lists[2];
    assert!(G_IND <= ls && ls <= le && (le - G_IND) as usize <= ind.len(), "tile list {ls:#x}..{le:#x}");
    Ours { bcl: lists[0].clone(), tiles: ind[(ls - G_IND) as usize..(le - G_IND) as usize].to_vec() }
}

/* ---- the shapes (tools/v3d/pi5/src/gl_shapes.py) ---------------------------- */

const COLOR_RT: V3dJob71Rt = V3dJob71Rt {
    address: G_RT0,
    format: V3D_OUTPUT_IMAGE_FORMAT_RGBA8,
    memory_format: V3D_MEMORY_FORMAT_UIF_NO_XOR,
    height_in_ub_or_stride: 8,
    r_b_swap: false,
    force_alpha_1: false,
    pad: [0; 2],
};

fn target(depth: bool) -> V3dJob71Target {
    let mut t = V3dJob71Target { width: W, height: H, tile_alloc: G_TILE, rts: 1, ..Default::default() };
    t.rt[0] = COLOR_RT;
    if depth {
        t.depth_type = V3D_INTERNAL_TYPE_DEPTH_24;
        t.zs = V3dJob71Zs {
            address: G_ZS,
            format: V3D_OUTPUT_IMAGE_FORMAT_D24S8,
            memory_format: V3D_MEMORY_FORMAT_UIF_NO_XOR,
            height_in_ub: 8,
        };
    }
    t
}

/// glClearColor(0, 0, 0, 1): RGBA8's word 0xff000000.
fn clear(depth: bool) -> V3dJob71Clear {
    let mut c = V3dJob71Clear { z: 1.0, ..Default::default() };
    c.color[0][0] = 0xff00_0000;
    if depth {
        // glClearDepthf(0.5), depth stored (gl_depthfuncs's job stores Z)
        c.z = 0.5;
        c.zs_store = true;
        c.zs_store_aspects = 1; // V3D_JOB71_ZS_DEPTH
        c.zs_cleared = true;
    }
    c
}

/// The window's state as gallium hands it over before any GL change: the
/// full 64x64 viewport (depth range 0..1), line width 1, every channel
/// written, blending and depth off, blend colour zero, no query.
fn base_state() -> V3dJob71State {
    V3dJob71State {
        vp: V3dViewport { scale: [32.0, 32.0, 0.5], translate: [32.0, 32.0, 0.5] },
        line_width: 1.0,
        depth_func: V3D_COMPARE_FUNC_LESS,
        colormask: 0xf,
        sample_mask: 0xf,
        ..Default::default()
    }
}

fn blendsub() -> Ours {
    let modes = [
        (
            V3D_BLEND_FACTOR_SRC_ALPHA,
            V3D_BLEND_FACTOR_ONE,
            V3D_BLEND_FACTOR_ONE,
            V3D_BLEND_FACTOR_ONE,
            V3D_BLEND_MODE_SUB,
            V3D_BLEND_MODE_SUB,
        ),
        (
            V3D_BLEND_FACTOR_ONE,
            V3D_BLEND_FACTOR_ONE,
            V3D_BLEND_FACTOR_ONE,
            V3D_BLEND_FACTOR_ONE,
            V3D_BLEND_MODE_RSUB,
            V3D_BLEND_MODE_RSUB,
        ),
        (
            V3D_BLEND_FACTOR_DST_COLOR,
            V3D_BLEND_FACTOR_ZERO,
            V3D_BLEND_FACTOR_ONE,
            V3D_BLEND_FACTOR_ONE,
            V3D_BLEND_MODE_ADD,
            V3D_BLEND_MODE_MIN,
        ),
        (
            V3D_BLEND_FACTOR_SRC_ALPHA_SATURATE,
            V3D_BLEND_FACTOR_INV_DST_ALPHA,
            V3D_BLEND_FACTOR_ONE,
            V3D_BLEND_FACTOR_ZERO,
            V3D_BLEND_MODE_SUB,
            V3D_BLEND_MODE_ADD,
        ),
    ];
    let draws: Vec<Draw> = modes
        .iter()
        .enumerate()
        .map(|(i, &(src, dst, asrc, adst, ceq, aeq))| {
            let mut st = base_state();
            st.blend_enabled = true;
            (st.rgb_src, st.rgb_dst, st.alpha_src, st.alpha_dst, st.rgb_func, st.alpha_func) =
                (src, dst, asrc, adst, ceq, aeq);
            Draw { st, prim: V3D_PRIM_TRIANGLES, count: 6, first: i as u32 * 6 }
        })
        .collect();
    build(&target(false), &clear(false), &draws)
}

fn depthfuncs() -> Ours {
    let funcs = [
        (V3D_COMPARE_FUNC_NEVER, true),
        (V3D_COMPARE_FUNC_EQUAL, true),
        (V3D_COMPARE_FUNC_LEQUAL, true),
        (V3D_COMPARE_FUNC_GREATER, true),
        (V3D_COMPARE_FUNC_NOTEQUAL, true),
        (V3D_COMPARE_FUNC_ALWAYS, true),
        (V3D_COMPARE_FUNC_LESS, false),
    ];
    let draws: Vec<Draw> = funcs
        .iter()
        .enumerate()
        .map(|(i, &(func, mask))| {
            let mut st = base_state();
            st.depth_enabled = true;
            st.depth_func = func;
            st.depth_writemask = mask;
            Draw { st, prim: V3D_PRIM_TRIANGLES, count: 6, first: i as u32 * 6 }
        })
        .collect();
    build(&target(true), &clear(true), &draws)
}

fn primswide() -> Ours {
    let seq = [(V3D_PRIM_POINTS, 1.0f32), (V3D_PRIM_LINES, 2.0), (V3D_PRIM_LINES, 4.0), (V3D_PRIM_LINE_STRIP, 2.0)];
    let draws: Vec<Draw> = seq
        .iter()
        .enumerate()
        .map(|(i, &(prim, width))| {
            let mut st = base_state();
            st.line_width = width;
            Draw { st, prim, count: 4, first: i as u32 * 4 }
        })
        .collect();
    build(&target(false), &clear(false), &draws)
}

const SHAPES: [&str; 3] = ["blendsub", "depthfuncs", "primswide"];

fn ours(shape: &str) -> &'static Ours {
    static O: OnceLock<BTreeMap<&'static str, Ours>> = OnceLock::new();
    &O.get_or_init(|| {
        BTreeMap::from([("blendsub", blendsub()), ("depthfuncs", depthfuncs()), ("primswide", primswide())])
    })[shape]
}

/* ---- the packet table of cle/v3d_packet.xml at version 71 ------------------- */

#[derive(Clone, Debug, PartialEq)]
struct Pk {
    name: String,
    fields: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
struct FieldDef {
    name: String,
    start: u32,
    size: u32,
    ty: String,
    minus_one: bool,
    default: Option<u64>,
}

#[derive(Clone, Debug)]
struct PacketDef {
    code: u8,
    name: String,
    fields: Vec<FieldDef>,
    len: usize,
}

fn attr(tag: &str, k: &str) -> Option<String> {
    let pat = format!(" {k}=\"");
    let i = tag.find(&pat)? + pat.len();
    let j = tag[i..].find('"')? + i;
    Some(tag[i..j].to_string())
}

fn ver_ok(tag: &str) -> bool {
    let min = attr(tag, "min_ver").map(|v| v.parse::<u32>().unwrap()).unwrap_or(0);
    let max = attr(tag, "max_ver").map(|v| v.parse::<u32>().unwrap()).unwrap_or(999);
    min <= 71 && 71 <= max
}

/// The CLIF name of a packet (Mesa's clif dump, v3d_decoder.c): its
/// shortname or name, upper case, each run of other characters one `_`.
fn clif_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn packet_defs() -> &'static [PacketDef] {
    static DEFS: OnceLock<Vec<PacketDef>> = OnceLock::new();
    DEFS.get_or_init(|| {
        let xml = read("third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml");
        let mut defs = vec![];
        let mut cur: Option<PacketDef> = None;
        for raw in xml.lines() {
            let line = raw.trim();
            if line.starts_with("<packet ") {
                if ver_ok(line) {
                    let name = attr(line, "shortname").or_else(|| attr(line, "name")).unwrap();
                    let code = attr(line, "code").unwrap().parse().unwrap();
                    let p = PacketDef { code, name: clif_name(&name), fields: vec![], len: 1 };
                    if line.ends_with("/>") {
                        defs.push(p);
                    } else {
                        cur = Some(p);
                    }
                }
                continue;
            }
            if line.starts_with("</packet>") {
                if let Some(mut p) = cur.take() {
                    let end = p.fields.iter().map(|f| f.start + f.size).max().unwrap_or(0);
                    p.len = 1 + end.div_ceil(8) as usize;
                    defs.push(p);
                }
                continue;
            }
            if let (Some(p), true) = (cur.as_mut(), line.starts_with("<field ")) {
                if !ver_ok(line) {
                    continue;
                }
                let start = attr(line, "start").unwrap();
                let start = match start.strip_suffix('b') {
                    Some(b) => b.parse::<u32>().unwrap() * 8,
                    None => start.parse().unwrap(),
                };
                p.fields.push(FieldDef {
                    name: attr(line, "name").unwrap(),
                    start,
                    size: attr(line, "size").unwrap().parse().unwrap(),
                    ty: attr(line, "type").unwrap(),
                    minus_one: attr(line, "minus_one").as_deref() == Some("true"),
                    default: attr(line, "default").map(|v| v.parse().unwrap()),
                });
            }
        }
        defs
    })
}

fn bits(body: &[u8], start: u32, size: u32) -> u64 {
    let mut v = 0u64;
    for i in 0..size {
        let b = start + i;
        if body[(b / 8) as usize] >> (b % 8) & 1 != 0 {
            v |= 1 << i;
        }
    }
    v
}

/// A field's value as Mesa's CLIF dump prints its first token; an address
/// is `[null]` or `[addr]` (a BO address is not compared).
fn clif_value(f: &FieldDef, raw: u64) -> String {
    let fl = |x: f64| format!("{x:.6}");
    let sext = |v: u64, n: u32| ((v << (64 - n)) as i64) >> (64 - n);
    match f.ty.as_str() {
        "address" if raw == 0 => "[null]".to_string(),
        "address" => "[addr]".to_string(),
        "bool" => format!("{}", raw & 1),
        "float" => fl(f64::from(f32::from_bits(raw as u32))),
        "f187" => fl(f64::from(f32::from_bits((raw as u32) << 16))),
        "s8.8" | "int" => format!("{}", sext(raw, f.size)),
        _ => format!("{}", raw + f.minus_one as u64),
    }
}

fn decode_list(bytes: &[u8], what: &str) -> Vec<Pk> {
    let defs = packet_defs();
    let mut out = vec![];
    let mut i = 0;
    while i < bytes.len() {
        let code = bytes[i];
        let cands: Vec<&PacketDef> = defs.iter().filter(|d| d.code == code).collect();
        let def = if cands.len() == 1 {
            cands[0]
        } else {
            let body = &bytes[i + 1..];
            *cands
                .iter()
                .find(|d| {
                    d.fields
                        .iter()
                        .filter(|f| f.default.is_some())
                        .all(|f| bits(body, f.start, f.size) == f.default.unwrap())
                })
                .unwrap_or_else(|| panic!("{what}: byte {i}: no 7.1 packet of code {code} matches"))
        };
        assert!(i + def.len <= bytes.len(), "{what}: {} at byte {i} runs past the list", def.name);
        let body = &bytes[i + 1..i + def.len];
        let fields = def.fields.iter().map(|f| (f.name.clone(), clif_value(f, bits(body, f.start, f.size)))).collect();
        out.push(Pk { name: def.name.clone(), fields });
        i += def.len;
    }
    out
}

/* ---- the references ------------------------------------------------------------ */

/// One reference job: its binning list and its other control lists (the
/// generic tile list among them), packets in file order.
struct RefJob {
    bcl: Vec<Pk>,
    tiles: Vec<Pk>,
}

fn field_line(line: &str) -> Option<(String, String)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("/*")?;
    let (label, value) = rest.split_once("*/")?;
    let label = label.trim().strip_suffix(':')?.trim().to_string();
    let value = value.split(" /*").next().unwrap_or("").trim();
    // an address is compared as null or not, never as a BO address
    let value = if value == "[null]" {
        value.to_string()
    } else if value.starts_with('[') {
        "[addr]".to_string()
    } else {
        value.split_whitespace().next().unwrap_or("").to_string()
    };
    Some((label, value))
}

fn is_packet(line: &str) -> bool {
    let b = line.as_bytes();
    !b.is_empty()
        && b[0].is_ascii_uppercase()
        && b.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
}

/// The dump's single job (each of the three shapes is one glFinish).
fn ref_job(shape: &str) -> RefJob {
    let text = read(&format!("tools/v3d/pi5/gl_{shape}.clif"));
    let mut lists: Vec<(String, Vec<Pk>)> = vec![];
    let (mut buf, mut in_list, mut bin) = (String::new(), false, None::<String>);
    let mut in_add_bin = false;
    let mut jobs = 0;
    for raw in text.lines() {
        let Some(line) = raw.strip_prefix("MESA: info:") else { continue };
        let line = line.strip_prefix(' ').unwrap_or(line);
        if let Some(b) = line.strip_prefix("@buffer ") {
            buf = b.split_whitespace().next().unwrap().to_string();
            in_list = false;
            in_add_bin = false;
            continue;
        }
        if let Some(f) = line.strip_prefix("@format ") {
            in_list = f.split_whitespace().next() == Some("ctrllist");
            in_add_bin = false;
            if in_list && !lists.iter().any(|l| l.0 == buf) {
                lists.push((buf.clone(), vec![]));
            }
            continue;
        }
        if let Some(k) = line.strip_prefix('@') {
            in_list = false;
            in_add_bin = k.starts_with("add_bin");
            if k.starts_with("wait_render") {
                jobs += 1;
            }
            continue;
        }
        if in_add_bin && bin.is_none() && line.trim_start().starts_with('[') {
            let a = line.trim().trim_start_matches('[');
            bin = Some(a.split(['+', ']']).next().unwrap().to_string());
            continue;
        }
        if !in_list {
            continue;
        }
        let l = lists.iter_mut().find(|l| l.0 == buf).unwrap();
        if is_packet(line) {
            l.1.push(Pk { name: line.to_string(), fields: vec![] });
        } else if let (Some(f), Some(p)) = (field_line(line), l.1.last_mut()) {
            p.fields.push(f);
        }
    }
    assert_eq!(jobs, 1, "{shape}: one job in the dump");
    let bin = bin.unwrap_or_else(|| panic!("{shape}: no @add_bin"));
    let bcl = lists.iter().find(|l| l.0 == bin).map(|l| l.1.clone()).unwrap_or_else(|| panic!("{shape}: no {bin}"));
    let tiles = lists.iter().filter(|l| l.0 != bin).flat_map(|l| l.1.clone()).collect();
    RefJob { bcl, tiles }
}

/* ---- the comparison --------------------------------------------------------- */

/// The binning-list packets compared at every draw: (our packet's CLIF name
/// from v3d_packet.xml, the name Mesa's dump prints). They are the same
/// packets; the dump prints some under another name.
const FIXED: [(&str, &str); 9] = [
    ("POINT_SIZE", "POINT_SIZE"),
    ("LINE_WIDTH", "LINE_WIDTH"),
    ("OCCLUSION_QUERY_COUNTER", "OCCLUSION_QUERY_COUNTER_ENABLE"),
    ("SAMPLE_STATE", "SAMPLE_STATE"),
    ("VCM_CACHE_SIZE", "VCM_CACHE_SIZE"),
    ("ZERO_ALL_FLAT_SHADE_FLAGS", "ZERO_ALL_FLATSHADE_FLAGS"),
    ("ZERO_ALL_NON_PERSPECTIVE_FLAGS", "ZERO_ALL_NOPERSPECTIVE_FLAGS"),
    ("ZERO_ALL_CENTROID_FLAGS", "ZERO_ALL_CENTROID_FLAGS"),
    ("BLEND_CONSTANT_COLOR", "BLEND_CCOLOR"),
];

/// The packet name the dump prints for one of ours.
fn dump_name(name: &str) -> String {
    FIXED.iter().find(|f| f.0 == name).map(|f| f.1.to_string()).unwrap_or_else(|| name.to_string())
}

const DRAWS: [&str; 4] =
    ["VERTEX_ARRAY_PRIMS", "VERTEX_ARRAY_INSTANCED_PRIMS", "INDEXED_PRIM_LIST", "INDEXED_INSTANCED_PRIM_LIST"];

/// (the prologue up to START_TILE_BINNING, the state in effect at each draw
/// by packet name).
fn split(ps: &[Pk]) -> (Vec<Pk>, Vec<BTreeMap<String, Pk>>) {
    let mut pro = vec![];
    let mut i = 0;
    while i < ps.len() {
        pro.push(ps[i].clone());
        i += 1;
        if ps[i - 1].name == "START_TILE_BINNING" {
            break;
        }
    }
    let mut state: BTreeMap<String, Pk> = BTreeMap::new();
    let mut draws = vec![];
    for p in &ps[i..] {
        if DRAWS.contains(&p.name.as_str()) {
            draws.push(state.clone());
        } else {
            state.insert(p.name.clone(), p.clone());
        }
    }
    (pro, draws)
}

/// Our binning list with the dump's packet names.
fn our_bcl(shape: &str) -> Vec<Pk> {
    decode_list(&ours(shape).bcl, &format!("{shape} bcl"))
        .into_iter()
        .map(|p| Pk { name: dump_name(&p.name), ..p })
        .collect()
}

fn our_tiles(shape: &str) -> Vec<Pk> {
    decode_list(&ours(shape).tiles, &format!("{shape} tiles"))
}

/// Field by field: a field on one side only, or a different value.
fn fld_cmp(w: &str, ours: &Pk, refp: &Pk, diffs: &mut Vec<String>) {
    for (k, v) in &refp.fields {
        match ours.fields.iter().find(|f| &f.0 == k) {
            Some((_, o)) if o == v => {}
            Some((_, o)) => diffs.push(format!("{w} {}.{k}: {o} != {v}", refp.name)),
            None => diffs.push(format!("{w} {}.{k}: absent != {v}", refp.name)),
        }
    }
    for (k, o) in &ours.fields {
        if !refp.fields.iter().any(|f| &f.0 == k) {
            diffs.push(format!("{w} {}.{k}: {o} != absent", refp.name));
        }
    }
}

/// Every value difference of one shape's compared packets, `ours != mesa`.
fn diffs(shape: &str) -> Vec<String> {
    let mut out = vec![];
    let r = ref_job(shape);
    let (opro, odraws) = split(&our_bcl(shape));
    let (rpro, rdraws) = split(&r.bcl);
    assert_eq!(odraws.len(), rdraws.len(), "{shape}: the draws");
    let oq = "OCCLUSION_QUERY_COUNTER_ENABLE";
    match (opro.iter().find(|p| p.name == oq), rpro.iter().find(|p| p.name == oq)) {
        (Some(o), Some(m)) => fld_cmp(&format!("{shape} prologue"), o, m, &mut out),
        (o, m) => out.push(format!("{shape} prologue {oq}: {} != {}", o.is_some(), m.is_some())),
    }
    for (i, (o, m)) in odraws.iter().zip(&rdraws).enumerate() {
        for (_, name) in FIXED {
            let w = format!("{shape} draw {i}");
            match (o.get(name), m.get(name)) {
                (Some(op), Some(mp)) => fld_cmp(&w, op, mp, &mut out),
                (None, None) => {}
                (op, mp) => out.push(format!(
                    "{w} {name}: {} != {}",
                    if op.is_some() { "present" } else { "absent" },
                    if mp.is_some() { "present" } else { "absent" }
                )),
            }
        }
    }
    let plf = "PRIM_LIST_FORMAT";
    let tiles = our_tiles(shape);
    let ot: Vec<&Pk> = tiles.iter().filter(|p| p.name == plf).collect();
    let mt: Vec<&Pk> = r.tiles.iter().filter(|p| p.name == plf).collect();
    if ot.len() != mt.len() {
        out.push(format!("{shape} tiles {plf}: {} packets != {}", ot.len(), mt.len()));
    }
    for (o, m) in ot.iter().zip(&mt) {
        fld_cmp(&format!("{shape} tiles"), o, m, &mut out);
    }
    out
}

/// A value difference the C job has today: (the difference, exactly as
/// [`diffs`] words it; the reason). Each has a `gap_*` test below.
const PINNED: [(&str, &str); 0] = [];

#[test]
fn harness_builds_every_shape() {
    for s in SHAPES {
        let o = ours(s);
        assert!(!o.bcl.is_empty() && !o.tiles.is_empty(), "{s}");
    }
}

#[test]
fn the_dumps_have_the_compared_packets() {
    // every compared packet occurs in at least one dump: a renamed packet
    // would otherwise compare absent with absent
    for (_, name) in FIXED {
        let n: usize = SHAPES.iter().map(|s| ref_job(s).bcl.iter().filter(|p| p.name == name).count()).sum();
        assert!(n > 0, "{name}: in no dump");
    }
    for s in SHAPES {
        assert!(ref_job(s).tiles.iter().any(|p| p.name == "PRIM_LIST_FORMAT"), "{s}: PRIM_LIST_FORMAT");
    }
}

#[test]
fn our_lists_have_the_compared_packets() {
    for s in SHAPES {
        let bcl = our_bcl(s);
        for (_, name) in FIXED {
            assert!(bcl.iter().any(|p| p.name == name), "{s}: {name} not in our binning list");
        }
        assert!(our_tiles(s).iter().any(|p| p.name == "PRIM_LIST_FORMAT"), "{s}: PRIM_LIST_FORMAT");
    }
}

#[test]
fn fixed_packets_match_mesa_or_are_pinned() {
    let mut all = vec![];
    for s in SHAPES {
        all.extend(diffs(s));
    }
    let unpinned: Vec<&String> = all.iter().filter(|d| !PINNED.iter().any(|p| p.0 == d.as_str())).collect();
    assert!(
        unpinned.is_empty(),
        "differences from Mesa not pinned:\n{}",
        unpinned.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
    );
    let stale: Vec<&str> = PINNED.iter().filter(|p| !all.iter().any(|d| d == p.0)).map(|p| p.0).collect();
    assert!(stale.is_empty(), "pins that no longer occur:\n{}", stale.join("\n"));
}

#[test]
fn the_comparison_sees_a_changed_value() {
    // the comparison is not vacuous: one changed field, one extra and one
    // missing field are each reported
    let st = &split(&our_bcl("primswide")).1[2];
    let lw = &st["LINE_WIDTH"];
    let mut other = lw.clone();
    other.fields[0].1 = "3.000000".into();
    let mut out = vec![];
    fld_cmp("w", lw, &other, &mut out);
    assert_eq!(out, ["w LINE_WIDTH.Line width: 4.000000 != 3.000000"]);
    let mut extra = lw.clone();
    extra.fields.push(("X".into(), "1".into()));
    out.clear();
    fld_cmp("w", lw, &extra, &mut out);
    fld_cmp("w", &extra, lw, &mut out);
    assert_eq!(out, ["w LINE_WIDTH.X: absent != 1", "w LINE_WIDTH.X: 1 != absent"]);
}

/* ---- the values, pinned explicitly (what the comparison saw) ------------------- */

fn at_draws(shape: &str, name: &str, field: &str) -> Vec<String> {
    split(&our_bcl(shape))
        .1
        .iter()
        .map(|st| st[name].fields.iter().find(|f| f.0 == field).unwrap_or_else(|| panic!("{name}.{field}")).1.clone())
        .collect()
}

#[test]
fn line_width_follows_gl_line_width() {
    // gl_primswide: points (GL's initial 1.0), lines 2.0, lines 4.0, strip 2.0
    assert_eq!(at_draws("primswide", "LINE_WIDTH", "Line width"), ["1.000000", "2.000000", "4.000000", "2.000000"]);
    assert_eq!(at_draws("blendsub", "LINE_WIDTH", "Line width"), ["1.000000"; 4]);
}

#[test]
fn point_size_is_gallium_one() {
    // ES has no glPointSize: the rasterizer's 1.0; gl_PointSize 16 comes
    // from the shaded vertex data, not from this packet
    for s in SHAPES {
        assert!(at_draws(s, "POINT_SIZE", "Point Size").iter().all(|v| v == "1.000000"), "{s}");
    }
}

#[test]
fn sample_state_and_query_are_fixed() {
    for s in SHAPES {
        assert!(at_draws(s, "SAMPLE_STATE", "Mask").iter().all(|v| v == "15"), "{s}");
        assert!(at_draws(s, "SAMPLE_STATE", "Coverage").iter().all(|v| v == "1.000000"), "{s}");
        assert!(at_draws(s, "OCCLUSION_QUERY_COUNTER_ENABLE", "address").iter().all(|v| v == "[null]"), "{s}");
    }
}

#[test]
fn vcm_cache_size_is_the_programs() {
    for s in SHAPES {
        let bin = at_draws(s, "VCM_CACHE_SIZE", "Number of 16-vertex batches for binning");
        let ren = at_draws(s, "VCM_CACHE_SIZE", "Number of 16-vertex batches for rendering");
        assert!(bin.iter().all(|v| *v == d("CS_VCM_CACHE_SIZE").to_string()), "{s}: {bin:?}");
        assert!(ren.iter().all(|v| *v == d("VS_VCM_CACHE_SIZE").to_string()), "{s}: {ren:?}");
    }
}

#[test]
fn blend_constant_colour_is_zero() {
    for s in SHAPES {
        for c in ["Red (F16)", "Green (F16)", "Blue (F16)", "Alpha (F16)"] {
            assert!(at_draws(s, "BLEND_CCOLOR", c).iter().all(|v| v == "0"), "{s}: {c}");
        }
    }
}

#[test]
fn prim_list_format_is_list_triangles() {
    for s in SHAPES {
        let t = our_tiles(s);
        let p = t.iter().find(|p| p.name == "PRIM_LIST_FORMAT").unwrap();
        let mut f = p.fields.clone();
        f.sort();
        // 2: List Triangles, whatever the draws' primitives (Mesa's
        // v3d_rcl_emit_generic_per_tile_list)
        assert_eq!(
            f,
            [("primitive type".to_string(), "2".to_string()), ("tri strip or fan".to_string(), "0".to_string())],
            "{s}"
        );
    }
}

/* ---- the emission difference: Mesa's dirty bits against every draw ------------ */

/// (shape, packet as the dump names it, our count in the binning list,
/// Mesa's): v3d_job71's `emit_state` writes every state packet on every draw,
/// Mesa only the dirty ones (v3dx_emit.c v3dX(emit_state)). The prologue's
/// OCCLUSION_QUERY_COUNTER (both sides write one) is counted too.
const EMISSIONS: [(&str, &str, usize, usize); 27] = [
    ("blendsub", "POINT_SIZE", 4, 1),
    ("blendsub", "LINE_WIDTH", 4, 1),
    ("blendsub", "OCCLUSION_QUERY_COUNTER_ENABLE", 5, 2),
    ("blendsub", "SAMPLE_STATE", 4, 1),
    ("blendsub", "VCM_CACHE_SIZE", 4, 1),
    ("blendsub", "ZERO_ALL_FLATSHADE_FLAGS", 4, 1),
    ("blendsub", "ZERO_ALL_NOPERSPECTIVE_FLAGS", 4, 1),
    ("blendsub", "ZERO_ALL_CENTROID_FLAGS", 4, 1),
    ("blendsub", "BLEND_CCOLOR", 4, 1),
    ("depthfuncs", "POINT_SIZE", 7, 1),
    ("depthfuncs", "LINE_WIDTH", 7, 1),
    ("depthfuncs", "OCCLUSION_QUERY_COUNTER_ENABLE", 8, 2),
    ("depthfuncs", "SAMPLE_STATE", 7, 1),
    ("depthfuncs", "VCM_CACHE_SIZE", 7, 1),
    ("depthfuncs", "ZERO_ALL_FLATSHADE_FLAGS", 7, 1),
    ("depthfuncs", "ZERO_ALL_NOPERSPECTIVE_FLAGS", 7, 1),
    ("depthfuncs", "ZERO_ALL_CENTROID_FLAGS", 7, 1),
    ("depthfuncs", "BLEND_CCOLOR", 7, 1),
    ("primswide", "POINT_SIZE", 4, 4),
    ("primswide", "LINE_WIDTH", 4, 4),
    ("primswide", "OCCLUSION_QUERY_COUNTER_ENABLE", 5, 2),
    ("primswide", "SAMPLE_STATE", 4, 1),
    ("primswide", "VCM_CACHE_SIZE", 4, 4),
    ("primswide", "ZERO_ALL_FLATSHADE_FLAGS", 4, 2),
    ("primswide", "ZERO_ALL_NOPERSPECTIVE_FLAGS", 4, 2),
    ("primswide", "ZERO_ALL_CENTROID_FLAGS", 4, 2),
    ("primswide", "BLEND_CCOLOR", 4, 1),
];

#[test]
fn emission_counts_are_pinned() {
    let mut seen = vec![];
    for s in SHAPES {
        let (o, m) = (our_bcl(s), ref_job(s).bcl);
        for (_, name) in FIXED {
            let c = |l: &[Pk]| l.iter().filter(|p| p.name == name).count();
            seen.push((s, name, c(&o), c(&m)));
        }
    }
    let want: Vec<(&str, &str, usize, usize)> = EMISSIONS.to_vec();
    assert_eq!(seen, want, "the emission counts (ours, Mesa's) changed");
}
