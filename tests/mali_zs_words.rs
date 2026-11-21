// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth/stencil descriptor fields of the cause-5 sequences through the GL
//! core on the Mali mock, field for field against Mesa 26.2.1's drm-shim
//! traces (zs-words, plan #373 cause 5).
//!
//! `tools/mali/testdata/pan_zs_ref/` holds Mesa's frames of the
//! `fbo.render.{shared_depthbuffer,recreate_depthbuffer,recreate_stencilbuffer}`
//! sequences for an RGB565 renderbuffer (tools/mali/pan_zs_ref.sh). This test
//! replays each sequence as zs_seq.py made it (the GL calls of its trace's
//! header, 128x128) through the GL API into the real Mali back end over a
//! mock of its OSAL (every buffer kept, every submit recorded), decodes what
//! the GPU would read (each RUN_IDVS's Depth/stencil descriptor, each
//! RUN_FRAGMENT's Framebuffer Parameters and ZS/CRC extension) and compares:
//!
//! - the frames and their draws: as many as Mesa's;
//! - per frame: `Has ZS CRC Extension`; the depth/stencil storage
//!   (`Z Internal Format`, the ZS target's `Write Format`); the store
//!   (`Z Write Enable`, `S Write Enable`, the preload/unload flags); whether
//!   the depth/stencil starts from memory (`Pre Frame 1`, EARLY_ZS_ALWAYS,
//!   the Valhall preload) or from its clear values, and then the clear
//!   values (`Z Clear`, `S Clear`); the ZS target's `Clean Tile Write
//!   Enable`;
//! - per draw: the whole Depth/stencil descriptor (compare functions, the
//!   three operations, masks, reference, depth function and writes).
//!
//! Addresses (FBD, ZS base, frame shader DCDs, tiler) differ by nature and
//! are not compared. Where the frames are cut differently, each sequence's
//! `Match` table says which Mesa frame and draw each of ours stands for
//! (the test's comments give the reason per sequence), and every Mesa draw
//! must be matched. The named differences the test pins (none changes a
//! pixel):
//!
//! - Frame cuts: ours makes no frame of a whole clear with no draw
//!   (shared_depthbuffer, Mesa's frame1); ours ends the frame at an unbind
//!   or a deleted attachment and loads the colour in the next one where
//!   Mesa goes on in one frame (the `no_rebind` sequences); a whole depth
//!   clear after the unbind is our frame's `Z Clear` where Mesa draws a
//!   clear quad, and a whole stencil clear after a held scissored clear is a
//!   clear quad in ours where it is Mesa's `S Clear`. A frame of ours that
//!   uses no depth/stencil (the stencil-only framebuffer's first quad) has
//!   no ZS/CRC extension, Mesa's has one.
//! - D16: the back end keeps every depth/stencil attachment pair in one
//!   linear D24S8 buffer (docs/notes/mali-zs-sharing.md section 1), so a
//!   `GL_DEPTH_COMPONENT16` renderbuffer is drawn as `Z Internal Format`
//!   D24 / ZS `D24S8` where Mesa's resource is D16. ES 2.0 4.4.3 lets the
//!   storage have more bits than asked; the depth compare is at least as
//!   exact. `STENCIL_INDEX8` alone is D24S8 in Mesa too.
//! - The `*_depth_component16_stencil_index8` sequences (the 35 cases): Mesa
//!   answers `GL_FRAMEBUFFER_UNSUPPORTED` (separate depth and stencil
//!   buffers, fbobject.c:1248-1253) and its traces hold no frame; the GL core
//!   of the `report-es2` build reports them complete and draws them. There
//!   is no Mesa frame to compare; the test pins Mesa's `# Frames: 0` and the
//!   GL core's completeness, and compares each frame of ours with the rule
//!   the single-attachment traces show (depth stored, D24S8, the preload
//!   when the frame does not clear what it uses).
//!
//! The feature set is `tests/mali_zs_sharing.rs`'s: the D16 + S8 pair is
//! complete only with `report-es2`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,report-es2 --test mali_zs_words
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::cs::Instr;
use yakogl::mali::genxml::v10::*;
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

macro_rules! trace {
    ($name:literal) => {
        ($name, include_str!(concat!("../tools/mali/testdata/pan_zs_ref/", $name, ".txt")))
    };
}

/* ---- the Mali over a mock OSAL ------------------------------------------ */

/// The OSAL's answers: the board's (panthor-frame on the T6).
const GPU: OsalMaliGpu =
    OsalMaliGpu { gpu_id: 0xa867_0005, tiler_features: 0x809, cs_reg_count: 96, unpreserved_cs_reg_count: 4 };
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

/// Buffers made so far: each one's GPU VA is its number times 16 MiB, far
/// from any CPU address. Nothing is given back (some MiB for the run).
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FENCES: AtomicI64 = AtomicI64::new(0);
/// Every buffer: (GPU VA, CPU address, size).
static BUFFERS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());
/// Every submit: the stream's GPU VA and length in bytes.
static KICKS: Mutex<Vec<(u64, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    let gpu = 0x40_0000_0000 + n * (16 << 20);
    BUFFERS.lock().unwrap().push((gpu, cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: n as *mut c_void };
    0
}
unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
    *mem = OsalGpuMem::ZERO;
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}
unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    assert_eq!(len, core::mem::size_of::<OsalMaliKick>());
    let k = &*kick.cast::<OsalMaliKick>();
    KICKS.lock().unwrap().push((k.stream_gpu, k.stream_bytes as usize));
    FENCES.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "C" fn m_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn m_completed() -> i64 {
    FENCES.load(Ordering::Relaxed)
}
unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    match what {
        OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
        OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => *out.cast::<OsalMaliHeap>() = HEAP,
        _ => return -ENOTSUP,
    }
    0
}

/// The OSAL's log hook: the library only calls it as `log("%s", line)`, so a
/// two-pointer function takes its place (the same calling convention for
/// these arguments).
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    eprintln!("osal log: {}", core::ffi::CStr::from_ptr(line).to_string_lossy());
}

static OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_alloc),
    gpu_free: Some(m_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: Some(m_submit),
    wait: Some(m_wait),
    completed: Some(m_completed),
    reg_read: None,
    reg_write: None,
    query: Some(m_query),
    // SAFETY: see `m_log`.
    log: Some(unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(*const c_char, *const c_char),
            unsafe extern "C" fn(*const c_char, ...),
        >(m_log)
    }),
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// `len` bytes of the buffer holding GPU address `gpu`.
fn mem(gpu: u64, len: usize) -> Vec<u8> {
    let b = BUFFERS.lock().unwrap();
    let &(base, cpu, _) = b
        .iter()
        .find(|&&(base, _, size)| gpu >= base && gpu + len as u64 <= base + size as u64)
        .unwrap_or_else(|| panic!("no buffer holds 0x{gpu:x}+{len}"));
    // SAFETY: the buffer is leaked, `size` bytes long, and the range is inside it.
    unsafe { core::slice::from_raw_parts((cpu + (gpu - base) as usize) as *const u8, len).to_vec() }
}

/* ---- what the back end built --------------------------------------------- */

/// One frame's depth/stencil-relevant words, ours or Mesa's.
#[derive(Debug)]
struct Frame {
    draws: Vec<DepthStencil>,
    params: FramebufferParameters,
    zs: Option<ZsTarget>,
}

/// The frames of the kicks since `from`, read as the GPU would: the stream's
/// register moves are followed, each RUN_IDVS reads the Depth/stencil
/// pointer, RUN_FRAGMENT the framebuffer descriptor and its ZS/CRC extension.
fn frames(from: usize) -> Vec<Frame> {
    let kicks = KICKS.lock().unwrap()[from..].to_vec();
    let mut out = Vec::new();
    for (gpu, bytes) in kicks {
        let raw = mem(gpu, bytes);
        let mut regs = [0u32; 128];
        let reg64 = |regs: &[u32; 128], r: usize| u64::from(regs[r]) | u64::from(regs[r + 1]) << 32;
        let mut draws = Vec::new();
        for w in raw.as_chunks::<8>().0 {
            match Instr::decode(u64::from_le_bytes(*w)) {
                Ok(Instr::Move48(m)) => {
                    regs[m.destination as usize] = m.immediate as u32;
                    regs[m.destination as usize + 1] = (m.immediate >> 32) as u32;
                }
                Ok(Instr::Move32(m)) => regs[m.destination as usize] = m.immediate,
                Ok(Instr::RunIdvs(_)) => {
                    let zsd = reg64(&regs, 52);
                    draws.push(DepthStencil::unpack(&mem(zsd, DepthStencil::LENGTH).try_into().unwrap()).unwrap());
                }
                Ok(Instr::RunFragment(_)) => {
                    let p = reg64(&regs, 40);
                    let fbd = p & !0x3f;
                    let params = FramebufferParameters::unpack(&mem(fbd, 64).try_into().unwrap()).unwrap();
                    let zs = (p & 2 != 0).then(|| {
                        let at = fbd + Framebuffer::LENGTH as u64;
                        ZsTarget::unpack(&mem(at, ZsTarget::LENGTH).try_into().unwrap()).unwrap()
                    });
                    out.push(Frame { draws: core::mem::take(&mut draws), params, zs });
                }
                _ => {}
            }
        }
    }
    out
}

/* ---- Mesa's trace --------------------------------------------------------- */

/// The value of `key` in the trace's block `block` (the first block of that
/// name that has it: a frame's `fbd` comes in two blocks).
fn ref_field<'a>(trace: &'a str, block: &str, key: &str) -> &'a str {
    let mut inside = false;
    for line in trace.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            inside = h.starts_with(&format!("{block} lines "));
            continue;
        }
        if let Some(v) = line.trim().strip_prefix(key).and_then(|r| r.strip_prefix(": ")).filter(|_| inside) {
            return v;
        }
    }
    panic!("{block}: no field {key}")
}

/// The value of `key` under the sub-block `sub` (`ZS:`, `S:`) of `block`.
fn ref_sub_field<'a>(trace: &'a str, block: &str, sub: &str, key: &str) -> &'a str {
    let (mut inside, mut in_sub) = (false, false);
    for line in trace.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            inside = h.starts_with(&format!("{block} lines "));
            in_sub = false;
            continue;
        }
        if !inside {
            continue;
        }
        if line.starts_with("  ") && !line.starts_with("    ") {
            in_sub = line.trim() == sub;
            continue;
        }
        if let Some(v) = line.trim().strip_prefix(key).and_then(|r| r.strip_prefix(": ")).filter(|_| in_sub) {
            return v;
        }
    }
    panic!("{block}: no field {sub} {key}")
}

fn func_of(v: &str) -> Func {
    match v {
        "Never" => Func::NEVER,
        "Less" => Func::LESS,
        "Equal" => Func::EQUAL,
        "Lequal" => Func::LEQUAL,
        "Greater" => Func::GREATER,
        "Not Equal" => Func::NOT_EQUAL,
        "Gequal" => Func::GEQUAL,
        "Always" => Func::ALWAYS,
        v => panic!("compare function {v}"),
    }
}

fn op_of(v: &str) -> StencilOp {
    match v {
        "Keep" => StencilOp::KEEP,
        "Replace" => StencilOp::REPLACE,
        "Zero" => StencilOp::ZERO,
        v => panic!("stencil op {v}"),
    }
}

fn pre_frame_of(v: &str) -> PrePostFrameShaderMode {
    match v {
        "Never" => PrePostFrameShaderMode::NEVER,
        "Always" => PrePostFrameShaderMode::ALWAYS,
        "Intersect" => PrePostFrameShaderMode::INTERSECT,
        "Early ZS Always" => PrePostFrameShaderMode::EARLY_ZS_ALWAYS,
        v => panic!("pre frame {v}"),
    }
}

/// The Depth/stencil descriptor pandecode printed in `block`.
fn ref_depth_stencil(trace: &str, block: &str) -> DepthStencil {
    let f = |k: &str| ref_field(trace, block, k);
    let hex = |k: &str| u32::from_str_radix(f(k).trim_start_matches("0x"), 16).unwrap();
    let flag = |k: &str| f(k) == "true";
    assert_eq!(
        (f("Depth clamp mode"), f("Depth source"), f("Depth units"), f("Depth factor"), f("Depth bias clamp")),
        ("[0, 1]", "Fixed function", "0.000000", "0.000000", "0.000000"),
        "{block}"
    );
    DepthStencil {
        front_compare_function: func_of(f("Front compare function")),
        front_stencil_fail: op_of(f("Front stencil fail")),
        front_depth_fail: op_of(f("Front depth fail")),
        front_depth_pass: op_of(f("Front depth pass")),
        back_compare_function: func_of(f("Back compare function")),
        back_stencil_fail: op_of(f("Back stencil fail")),
        back_depth_fail: op_of(f("Back depth fail")),
        back_depth_pass: op_of(f("Back depth pass")),
        stencil_from_shader: flag("Stencil from shader"),
        stencil_test_enable: flag("Stencil test enable"),
        front_write_mask: hex("Front write mask"),
        back_write_mask: hex("Back write mask"),
        front_value_mask: hex("Front value mask"),
        back_value_mask: hex("Back value mask"),
        front_reference_value: hex("Front reference value"),
        back_reference_value: hex("Back reference value"),
        shader_read_only_z_s: flag("Shader read only Z/S"),
        depth_cull_enable: flag("Depth cull enable"),
        depth_write_enable: flag("Depth write enable"),
        depth_bias_enable: flag("Depth bias enable"),
        depth_function: func_of(f("Depth function")),
        ..DepthStencil::default()
    }
}

/// The number of frames the trace says it holds (`# Frames: N`).
fn ref_frame_count(trace: &str) -> usize {
    trace.lines().find_map(|l| l.strip_prefix("# Frames: ")).expect("# Frames").parse().unwrap()
}

/// The number of draws of frame `n` (1-based) in the trace's summary line.
fn ref_draw_count(trace: &str, n: usize) -> usize {
    let head = format!("#   frame{n}: ");
    let l = trace.lines().find(|l| l.starts_with(&head)).expect("frame summary");
    let at = l.find(" draw(s)").unwrap();
    let start = l[..at].rfind(' ').unwrap() + 1;
    l[start..at].parse().unwrap()
}

/// Mesa's frame `n` (1-based): its Framebuffer Parameters and ZS target
/// fields that the comparison reads, and its draws' Depth/stencil.
fn ref_frame(trace: &str, n: usize) -> Frame {
    let fbd = format!("frame{n}-fbd");
    let f = |k: &str| ref_field(trace, &fbd, k);
    let flag = |k: &str| f(k) == "true";
    let params = FramebufferParameters {
        pre_frame_0: pre_frame_of(f("Pre Frame 0")),
        pre_frame_1: pre_frame_of(f("Pre Frame 1")),
        s_clear: f("S Clear").parse().unwrap(),
        s_write_enable: flag("S Write Enable"),
        s_preload_enable: flag("S Preload Enable"),
        s_unload_enable: flag("S Unload Enable"),
        z_internal_format: match f("Z Internal Format") {
            "D16" => ZInternalFormat::D16,
            "D24" => ZInternalFormat::D24,
            v => panic!("{fbd}: Z Internal Format {v}"),
        },
        z_write_enable: flag("Z Write Enable"),
        z_preload_enable: flag("Z Preload Enable"),
        z_unload_enable: flag("Z Unload Enable"),
        has_zs_crc_extension: flag("Has ZS CRC Extension"),
        z_clear: f("Z Clear").parse().unwrap(),
        ..FramebufferParameters::default()
    };
    let zs = params.has_zs_crc_extension.then(|| {
        let z = |k: &str| ref_sub_field(trace, &fbd, "ZS:", k);
        assert_eq!(ref_sub_field(trace, &fbd, "S:", "Block Format"), "No Write", "{fbd}: no S plane");
        ZsTarget {
            write_format: match z("Write Format") {
                "D16" => ZsFormat::D16,
                "D24S8" => ZsFormat::D24S8,
                v => panic!("{fbd}: ZS Write Format {v}"),
            },
            block_format: BlockFormat::LINEAR,
            msaa: Msaa::SINGLE,
            clean_tile_write_enable: z("Clean Tile Write Enable") == "true",
            ..Default::default()
        }
    });
    let draws =
        (1..=ref_draw_count(trace, n)).map(|d| ref_depth_stencil(trace, &format!("frame{n}-draw{d}"))).collect();
    Frame { draws, params, zs }
}

/* ---- the comparison ------------------------------------------------------- */

/// How one of our frames lines up with Mesa's.
struct Match {
    /// Mesa's frame (1-based) whose framebuffer fields ours must equal, or
    /// 0: ours has no depth/stencil (the frame uses none).
    mesa: usize,
    /// Ours ends the frame where Mesa's goes on (an unbind), so ours starts
    /// the colour from memory where Mesa's continues its frame: `Pre Frame 0`
    /// is INTERSECT in ours and not compared.
    split: bool,
    /// Each of our draws: Mesa's (frame, draw), both 1-based, or (0, 0) for
    /// a whole stencil clear that is a quad in ours and the frame's `S Clear`
    /// in Mesa's.
    draws: &'static [(usize, usize)],
}

const fn m(mesa: usize, split: bool, draws: &'static [(usize, usize)]) -> Match {
    Match { mesa, split, draws }
}

/// Our frame `n` against the Mesa frame `map` names, field by field. `d16`:
/// Mesa's depth is a D16 resource, ours the pair's D24S8 (the named storage
/// difference); `depth`: the framebuffer has a depth attachment (else
/// `Z Clear` means nothing and is not compared).
fn compare(name: &str, n: usize, ours: &Frame, trace: &str, map: &Match, d16: bool, depth: bool) {
    let at = format!("{name} frame{n}");
    let o = &ours.params;
    assert!(ours.zs.is_some() == o.has_zs_crc_extension, "{at}: pointer and parameters agree");
    if map.mesa == 0 {
        /* a frame of ours that reads and writes no depth/stencil: no ZS/CRC
         * extension; the planes keep their values outside the frame */
        assert!(!o.has_zs_crc_extension, "{at}: no ZS/CRC extension");
    } else {
        let mesa = ref_frame(trace, map.mesa);
        let mp = &mesa.params;
        assert_eq!(o.has_zs_crc_extension, mp.has_zs_crc_extension, "{at}: Has ZS CRC Extension");
        /* storage: Mesa's D16 is our D24 / D24S8; otherwise the same */
        assert_eq!(mp.z_internal_format, if d16 { ZInternalFormat::D16 } else { ZInternalFormat::D24 }, "{at}");
        assert_eq!(o.z_internal_format, ZInternalFormat::D24, "{at}: Z Internal Format (D24S8 pair buffer)");
        let (oz, mz) = (ours.zs.unwrap(), mesa.zs.unwrap());
        assert_eq!(mz.write_format, if d16 { ZsFormat::D16 } else { ZsFormat::D24S8 }, "{at}");
        assert_eq!(oz.write_format, ZsFormat::D24S8, "{at}: ZS Write Format (D24S8 pair buffer)");
        assert_eq!(oz.block_format, mz.block_format, "{at}: ZS Block Format");
        assert_eq!(oz.msaa, mz.msaa, "{at}: ZS MSAA");
        assert_eq!(oz.clean_tile_write_enable, mz.clean_tile_write_enable, "{at}: ZS Clean Tile Write Enable");
        /* the store, and the preload/unload flags Valhall leaves off */
        assert_eq!(o.z_write_enable, mp.z_write_enable, "{at}: Z Write Enable");
        assert_eq!(o.s_write_enable, mp.s_write_enable, "{at}: S Write Enable");
        assert_eq!(
            (o.z_preload_enable, o.z_unload_enable, o.s_preload_enable, o.s_unload_enable),
            (mp.z_preload_enable, mp.z_unload_enable, mp.s_preload_enable, mp.s_unload_enable),
            "{at}: Z/S preload and unload flags"
        );
        /* the Z/S start: preloaded by the pre-frame-1 draw, or the clear values */
        assert_eq!(o.pre_frame_1, mp.pre_frame_1, "{at}: Pre Frame 1 (the Z/S preload)");
        if map.split {
            assert_eq!(o.pre_frame_0, PrePostFrameShaderMode::INTERSECT, "{at}: the split frame loads the colour");
        } else {
            /* a preload of the colour or none; INTERSECT and ALWAYS load
             * the same image (Mesa: ALWAYS when it reloads after a
             * mid-frame flush, pan_fb_preload.c:1194-1201) */
            assert_eq!(
                o.pre_frame_0 == PrePostFrameShaderMode::NEVER,
                mp.pre_frame_0 == PrePostFrameShaderMode::NEVER,
                "{at}: Pre Frame 0 (the colour preload)"
            );
        }
        if mp.pre_frame_1 == PrePostFrameShaderMode::NEVER {
            if depth {
                assert_eq!(o.z_clear, mp.z_clear, "{at}: Z Clear");
            }
            assert_eq!(o.s_clear, mp.s_clear, "{at}: S Clear");
        }
    }
    /* the draws */
    assert_eq!(ours.draws.len(), map.draws.len(), "{at}: draws");
    for (i, (od, &(mf, md))) in ours.draws.iter().zip(map.draws).enumerate() {
        if mf == 0 {
            /* a whole stencil clear after a draw of the frame: a quad that
             * replaces the stencil everywhere with the frame's S Clear, as
             * Mesa's scissored clear quad does in its box */
            assert!(
                od.stencil_test_enable
                    && od.front_compare_function == Func::ALWAYS
                    && od.front_depth_pass == StencilOp::REPLACE
                    && od.front_write_mask == 0xff
                    && od.front_reference_value == o.s_clear
                    && !od.depth_write_enable,
                "{at} draw{}: the whole stencil clear quad: {od:?}",
                i + 1
            );
            continue;
        }
        let want = &ref_frame(trace, mf).draws[md - 1];
        assert_eq!(od, want, "{at} draw{} (Mesa frame{mf} draw{md}): the Depth/stencil descriptor", i + 1);
    }
}

/* ---- the sequences, as zs_seq.py made them ------------------------------- */

const W: GLsizei = 128;

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

fn setup() -> GLuint {
    backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
    // SAFETY: static tables and one context; this is the only test of the
    // file.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
        assert_eq!(yakogl::mali::backend::mali_init(), 0);
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(W as u32, W as u32).is_null());
        /* the depth/stencil state does not depend on the program: one flat
         * colour program stands for SingleTex2DShader and FlatColorShader */
        let v = shader(GL_VERTEX_SHADER, "attribute vec4 a_position;\nvoid main() { gl_Position = a_position; }\n");
        let f = shader(
            GL_FRAGMENT_SHADER,
            "precision mediump float;\nuniform vec4 u_color;\nvoid main() { gl_FragColor = u_color; }\n",
        );
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
        glUniform4f(glGetUniformLocation(p, c"u_color".as_ptr()), 0.0, 1.0, 0.0, 1.0);
        while glGetError() != GL_NO_ERROR {}
        p
    }
}

fn ok(what: &str) {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}");
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/// zs_seq.py's drawQuad: corners z `z0` and `z1`, the middle ones their mean.
fn quad(z0: f32, z1: f32) {
    let mid = 0.5 * (z0 + z1);
    let p: [f32; 16] = [-1.0, -1.0, z0, 1.0, -1.0, 1.0, mid, 1.0, 1.0, -1.0, mid, 1.0, 1.0, 1.0, z1, 1.0];
    static IDX: [u16; 6] = [0, 2, 1, 2, 1, 3];
    let before = backend::draws_refused();
    // SAFETY: `p` and `IDX` outlive the draw: the draw's vertices are taken
    // before it returns (client arrays), and nothing reads them later.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, p.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
        glDisableVertexAttribArray(0);
    }
    ok("draw");
    assert_eq!(backend::draws_refused(), before, "a draw was refused");
}

fn renderbuffer(fmt: GLenum, name: GLuint) -> GLuint {
    let mut rb = name;
    // SAFETY: room for one name.
    unsafe {
        if rb == 0 {
            glGenRenderbuffers(1, &mut rb);
        }
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, fmt, W, W);
    }
    ok("renderbuffer");
    rb
}

fn attach(point: GLenum, rb: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb) };
    ok("attach");
}

fn bind(f: GLuint) {
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, f) };
    ok("bind");
}

struct Fbo {
    fbo: GLuint,
    depth: GLuint,
    stencil: GLuint,
}

/// zs_seq.py's Fbo: RGB565 colour, then depth, then stencil; left unbound.
fn fbo(depth: bool, stencil: bool) -> Fbo {
    let mut f = 0;
    // SAFETY: room for one name.
    unsafe { glGenFramebuffers(1, &mut f) };
    bind(f);
    attach(GL_COLOR_ATTACHMENT0, renderbuffer(GL_RGB565, 0));
    let mut r = Fbo { fbo: f, depth: 0, stencil: 0 };
    if depth {
        r.depth = renderbuffer(GL_DEPTH_COMPONENT16, 0);
        attach(GL_DEPTH_ATTACHMENT, r.depth);
    }
    if stencil {
        r.stencil = renderbuffer(GL_STENCIL_INDEX8, 0);
        attach(GL_STENCIL_ATTACHMENT, r.stencil);
    }
    bind(0);
    r
}

/// Fbo.complete: bind, check, unbind. The status, or None when complete.
fn complete(f: &Fbo) -> Option<GLenum> {
    bind(f.fbo);
    let s = status();
    bind(0);
    (s != GL_FRAMEBUFFER_COMPLETE).then_some(s)
}

const ALL: GLbitfield = GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT;

/// zs_seq.py's `shared_depthbuffer`. The frames it made since `from`.
fn shared_depthbuffer(stencil: bool) -> Vec<Frame> {
    let from = KICKS.lock().unwrap().len();
    // SAFETY: plain GL calls throughout.
    unsafe {
        glViewport(0, 0, W, W);
        let a = fbo(true, stencil);
        assert_eq!(complete(&a), None, "the GL core reports A complete");
        let b = fbo(false, stencil);
        bind(b.fbo);
        attach(GL_DEPTH_ATTACHMENT, a.depth);
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClearStencil(1);
        glClear(ALL);
        glEnable(GL_DEPTH_TEST);
        bind(a.fbo);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(ALL);
        quad(0.0, 0.0);
        bind(b.fbo);
        quad(-1.0, 1.0);
        glDisable(GL_DEPTH_TEST);
        if stencil {
            glEnable(GL_SCISSOR_TEST);
            glScissor(10, 10, 12, 25);
            glClearStencil(0);
            glClear(GL_STENCIL_BUFFER_BIT);
            glDisable(GL_SCISSOR_TEST);
            glEnable(GL_STENCIL_TEST);
            glStencilFunc(GL_EQUAL, 0, 0xff);
            quad(0.0, 0.0);
            glDisable(GL_STENCIL_TEST);
        }
        /* the case's read-back: glReadPixels ends the frame */
        glFinish();
        ok("shared_depthbuffer");
        bind(0);
    }
    frames(from)
}

/// zs_seq.py's `recreate`. The frames it made since `from`.
fn recreate(depth_buffers: bool, rebind: bool, depth: bool, stencil: bool) -> Vec<Frame> {
    let from = KICKS.lock().unwrap().len();
    // SAFETY: plain GL calls throughout.
    unsafe {
        let f = fbo(depth, stencil);
        assert_eq!(complete(&f), None, "the GL core reports the framebuffer complete");
        bind(f.fbo);
        glViewport(0, 0, W, W);
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(ALL);
        glEnable(GL_DEPTH_TEST);
        quad(0.0, 0.0);
        if stencil {
            glEnable(GL_SCISSOR_TEST);
            glScissor(W / 4, W / 4, W / 2, W / 2);
            glClearStencil(1);
            glClear(GL_STENCIL_BUFFER_BIT);
            glDisable(GL_SCISSOR_TEST);
        }
        if !rebind {
            bind(0);
        }
        let (point, fmt, name) = if depth_buffers {
            (GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, f.depth)
        } else {
            (GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8, f.stencil)
        };
        glDeleteRenderbuffers(1, &name);
        renderbuffer(fmt, name);
        if rebind {
            attach(point, name);
        } else {
            bind(f.fbo);
        }
        glClearColor(0.0, 0.0, 1.0, 0.0);
        glClearStencil(0);
        glClear(if depth_buffers { GL_DEPTH_BUFFER_BIT } else { GL_STENCIL_BUFFER_BIT });
        if stencil {
            glEnable(GL_STENCIL_TEST);
            glStencilFunc(GL_EQUAL, 0, 0xff);
        }
        quad(1.0, -1.0);
        if stencil {
            glDisable(GL_STENCIL_TEST);
        }
        glDisable(GL_DEPTH_TEST);
        glFinish();
        ok("recreate");
        bind(0);
    }
    frames(from)
}

/* ---- the traces ----------------------------------------------------------- */

/// A single-attachment sequence against its trace: our frames as `map`
/// lines them up, and every Mesa draw matched by one of ours, but for
/// `mesa_only` (Mesa draws that are a frame's clear value in ours).
fn check_traced(
    name: &str,
    trace: &str,
    ours: &[Frame],
    map: &[Match],
    mesa_only: &[(usize, usize)],
    d16: bool,
    depth: bool,
) {
    assert!(trace.contains("GL_FRAMEBUFFER_COMPLETE") && trace.contains(": drawn, glGetError 0x0"), "{name}");
    let summary: Vec<String> = ours
        .iter()
        .map(|f| format!("{} draw(s), pre {}/{}", f.draws.len(), f.params.pre_frame_0.0, f.params.pre_frame_1.0))
        .collect();
    assert_eq!(ours.len(), map.len(), "{name}: frames (ours: {summary:?})");
    for (i, (f, mp)) in ours.iter().zip(map).enumerate() {
        compare(name, i + 1, f, trace, mp, d16, depth);
    }
    let mut mesa_draws: Vec<(usize, usize)> = (1..=ref_frame_count(trace))
        .flat_map(|n| (1..=ref_draw_count(trace, n)).map(move |d| (n, d)))
        .filter(|d| !mesa_only.contains(d))
        .collect();
    let mut matched: Vec<(usize, usize)> =
        map.iter().flat_map(|mp| mp.draws.iter().copied()).filter(|&(f, _)| f != 0).collect();
    mesa_draws.sort();
    matched.sort();
    assert_eq!(matched, mesa_draws, "{name}: every Mesa draw has its draw of ours");
}

/// A D16 + S8 sequence: Mesa made no frame (UNSUPPORTED); ours are drawn.
/// Each of our frames with a depth/stencil stores both planes in the pair's
/// D24S8 buffer and writes clean tiles when it does not preload the Z/S (the
/// rule the single-attachment frames above share with Mesa).
fn check_pair(name: &str, trace: &str, ours: &[Frame]) {
    assert!(
        trace.contains("glCheckFramebufferStatus = 0x8cdd GL_FRAMEBUFFER_UNSUPPORTED") && ref_frame_count(trace) == 0,
        "{name}: Mesa's trace is NotSupported with no frame"
    );
    assert!(!ours.is_empty(), "{name}: the GL core draws the sequence");
    for (i, f) in ours.iter().enumerate() {
        let at = format!("{name} frame{}", i + 1);
        let p = &f.params;
        assert!(p.has_zs_crc_extension, "{at}: every frame of the pair uses its depth");
        assert_eq!(p.z_internal_format, ZInternalFormat::D24, "{at}");
        assert_eq!(f.zs.map(|z| z.write_format), Some(ZsFormat::D24S8), "{at}");
        assert!(p.z_write_enable && !p.s_write_enable, "{at}: the D24S8 store");
        if p.pre_frame_1 == PrePostFrameShaderMode::NEVER {
            assert!(f.zs.unwrap().clean_tile_write_enable, "{at}: clean tile writes without a Z/S preload");
        }
    }
}

#[test]
fn cause_5_depth_stencil_fields_equal_mesa_s() {
    setup();
    /* shared_depthbuffer, depth only: Mesa's frame1 is B's whole clear with
     * no draw; ours makes no frame of it (the GL core keeps a whole clear as
     * the planes' and the colour's value) and B's frame starts from it. Our
     * frames 1, 2 are Mesa's 2, 3 field for field. */
    let (name, trace) = trace!("shared_depthbuffer__rbo_rgb565_depth_component16");
    let ours = shared_depthbuffer(false);
    check_traced(name, trace, &ours, &[m(2, false, &[(2, 1)]), m(3, false, &[(3, 1)])], &[], true, true);
    /* recreate_depthbuffer.rebind: equal, frame for frame */
    let (name, trace) = trace!("recreate_depthbuffer__rebind_rbo_rgb565_depth_component16");
    let ours = recreate(true, true, true, false);
    check_traced(name, trace, &ours, &[m(1, false, &[(1, 1)]), m(2, false, &[(2, 1)])], &[], true, true);
    /* recreate_depthbuffer.no_rebind: Mesa keeps one frame across the
     * unbind and clears the depth with a quad (draw2: depth Always, write);
     * ours ends the frame at glBindFramebuffer(0), and the next one starts
     * from the depth clear value (Z Clear 1.0, compared) and the colour
     * loaded */
    let (name, trace) = trace!("recreate_depthbuffer__no_rebind_rbo_rgb565_depth_component16");
    let ours = recreate(true, false, true, false);
    let clear = ref_frame(trace, 1).draws[1];
    assert!(clear.depth_function == Func::ALWAYS && clear.depth_write_enable, "{name}: Mesa's depth clear quad");
    check_traced(name, trace, &ours, &[m(1, false, &[(1, 1)]), m(1, true, &[(1, 3)])], &[(1, 2)], true, true);
    /* recreate_stencilbuffer: the first quad uses no depth/stencil (no depth
     * attachment, stencil test off), so our first frame has no ZS/CRC
     * extension. The scissored stencil clear is held and opens our second
     * frame (Mesa: draw2 of its first); then the whole stencil clear is a
     * quad in ours (rebind: Mesa's frame2 S Clear; no_rebind: Mesa's quad,
     * draw3 of its one frame). */
    let (name, trace) = trace!("recreate_stencilbuffer__rebind_rbo_rgb565_stencil_index8");
    let ours = recreate(false, true, false, true);
    check_traced(
        name,
        trace,
        &ours,
        &[m(0, false, &[(1, 1)]), m(2, false, &[(1, 2), (0, 0), (2, 1)])],
        &[],
        false,
        false,
    );
    let (name, trace) = trace!("recreate_stencilbuffer__no_rebind_rbo_rgb565_stencil_index8");
    let ours = recreate(false, false, false, true);
    check_traced(
        name,
        trace,
        &ours,
        &[m(0, false, &[(1, 1)]), m(1, true, &[(1, 2), (1, 3), (1, 4)])],
        &[],
        false,
        false,
    );
    /* the 35 cases' sequences: no Mesa frame to compare */
    let (name, trace) = trace!("shared_depthbuffer__rbo_rgb565_depth_component16_stencil_index8");
    check_pair(name, trace, &shared_depthbuffer(true));
    let (name, trace) = trace!("recreate_depthbuffer__rebind_rbo_rgb565_depth_component16_stencil_index8");
    check_pair(name, trace, &recreate(true, true, true, true));
    let (name, trace) = trace!("recreate_depthbuffer__no_rebind_rbo_rgb565_depth_component16_stencil_index8");
    check_pair(name, trace, &recreate(true, false, true, true));
    let (name, trace) = trace!("recreate_stencilbuffer__rebind_rbo_rgb565_depth_component16_stencil_index8");
    check_pair(name, trace, &recreate(false, true, true, true));
    let (name, trace) = trace!("recreate_stencilbuffer__no_rebind_rbo_rgb565_depth_component16_stencil_index8");
    check_pair(name, trace, &recreate(false, false, true, true));
    assert_eq!(backend::draws_refused(), 0);
}
