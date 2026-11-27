// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES3 `fragment_out.array.float.{rgba16f,rg16f}_*` on the GC7000UL
//! (causes `gc-16f-target` and `gc-16f-attach-n`, 24 board Fails at campaign
//! commit 9bc82915, entry 18, `tools/cts/log/2026-11-imx8mp-gles3-9bc82915-rows17-34`),
//! on the rig of `backend_tests.rs`.
//!
//! Each case draws one output array into three colour attachments of one
//! format. RGBA16F, and RG16F and RGB16F, which the GL core stores as four
//! halves (`GLES_SURF_RGBA16F`, `fb_target_n`), are 8 bytes a pixel. The pixel
//! engine writes 64-bit pixels into a LINEAR surface at the wrong places
//! (gl_test case 44, `wide_target.rs`), so a single 64-bit target renders
//! into a tiled shadow, and `fragment_out.basic.float.rgba16f_*` passes on the
//! board. A draw of several did not: it kept every target linear, and the
//! three attachments came back with their pixel pairs moved. The host's GPU
//! model does not move them, so the cause read "board-only".
//!
//! What the back end holds now (`backend::wide_targets`): a draw of two to
//! four hardware targets, every one 64-bit and with a CPU view of its
//! storage, renders each into a page-aligned TILED shadow of its own (RT0
//! through PE_PIPE_COLOR_ADDR and PE_COLOR_STRIDE, RT1.. through the 8-RT
//! bank's address and RT_CONFIG stride), with PE_LOGIC_OP SINGLE_BUFFER(2)
//! for all of them, and each shadow comes back into its own storage when the
//! kick completes. The GL core hands over a CPU view of attachment 0 only
//! (`GlesDrawTarget` has no `cpu`), so these tests give the others through
//! `test_views`; without one the draw keeps the linear path, as at 9bc82915.

use super::*;
use crate::vivante::backend::test_views;
use crate::vivante::wide_target::{self as wt, Target};

/// Bytes of an 8x8 64-bit storage.
const BYTES: usize = 8 * 8 * 8;
/// A shadow row: the 8-pixel width padded to 16, 8 bytes a pixel.
const SHADOW_STRIDE: u32 = 16 * 8;

/// `n` 8x8 storages of `format` (8 bytes a pixel, rows of 64 bytes), each
/// filled with its own pixel values, and a draw of one fragment output a
/// target into them (output k to target k, as dEQP's array draws).
/// `views`: attachments 1.. get a CPU view in `test_views`.
fn wide_mrt(rig: &Rig, n: usize, format: u8, views: bool) -> (Vec<OsalGpuMem>, GlesDraw, Box<ShcShader>) {
    test_views::clear();
    let mut mems = Vec::new();
    for k in 0..n {
        let mut mem = OsalGpuMem::ZERO;
        assert_eq!(osal::gpu_alloc(GC_HEAP_DATA, BYTES, OSAL_MEM_DATA, &mut mem), 0);
        assert!(!mem.cpu.is_null());
        fill(&mem, k);
        mems.push(mem);
    }
    let mut fs = Box::new(*rig.fs);
    fs.viv.ps_color_out_reg = 3;
    fs.viv.num_out = n as u8;
    for k in 0..n {
        fs.viv.out[k] = ShcVivIo { reg: 3 + k as u8, slot: 4 + k as u8, components: 4, interp: 0, name: [0; 32] };
    }
    let mut d = rig.draw(3);
    (d.target_gpu, d.target_cpu, d.target_w, d.target_h) = (mems[0].gpu, mems[0].cpu, 8, 8);
    d.target_format = format;
    d.viewport = [0, 0, 8, 8];
    (d.ds_w, d.ds_h) = (8, 8);
    d.num_targets = n as u8;
    for (k, m) in mems.iter().enumerate() {
        d.targets[k] = GlesDrawTarget { gpu: m.gpu, w: 8, h: 8, format, out_shift: 0 };
        if k > 0 && views {
            test_views::set(m.gpu, m.cpu as *mut u8);
        }
    }
    d.fs = &*fs;
    (mems, d, fs)
}

/// Storage `k`'s pixel `i`: its own value.
fn px(k: usize, i: usize) -> u64 {
    0x0001_0000_0000_0000 * (k as u64 + 1) + 0x1000 + i as u64
}

fn fill(mem: &OsalGpuMem, k: usize) {
    // SAFETY: the BYTES just allocated with a CPU view.
    let p = unsafe { core::slice::from_raw_parts_mut(mem.cpu as *mut u64, BYTES / 8) };
    for (i, v) in p.iter_mut().enumerate() {
        *v = px(k, i);
    }
}

fn pixels(mem: &OsalGpuMem) -> Vec<u64> {
    // SAFETY: the BYTES `wide_mrt` allocated.
    unsafe { core::slice::from_raw_parts(mem.cpu as *const u64, BYTES / 8) }.to_vec()
}

fn free(mems: &mut [OsalGpuMem]) {
    test_views::clear();
    for m in mems {
        osal::gpu_free(m);
    }
}

/// The SINGLE_BUFFER field of PE_LOGIC_OP.
fn single_buffer_of(s: &Stream) -> u32 {
    (s.reg(GC_PE_LOGIC_OP) >> 8) & 3
}

/// Which draws take the multi-target shadow: two or more hardware targets,
/// all of 8 bytes a pixel, each with a CPU view. One target, a narrower
/// target beside them or a missing view keeps what it had.
#[test]
fn several_64_bit_targets_with_cpu_views_take_one_shadow_each() {
    let rig = Rig::new();
    let (mut mems, d, _fs) = wide_mrt(&rig, 3, GLES_SURF_RGBA16F, true);
    let view = |slot: u8| {
        let m = &mems[slot as usize];
        Some((m.cpu as *mut u8, 64))
    };
    let w = wide_targets_of(&d, view).expect("three RGBA16F targets with views");
    assert_eq!(w.n, 3);
    for (k, t) in w.list().iter().enumerate() {
        assert_eq!(*t, Target { gpu: mems[k].gpu, cpu: mems[k].cpu as *mut u8, w: 8, h: 8, pitch: 64, bpp: 8 });
    }
    /* the views the draw itself asks for: slot 0 from target_cpu, the others
     * from test_views (target_pitch: 8 even pixels of 8 bytes) */
    assert_eq!(wide_targets(&d), Some(w));

    let mut one = d;
    one.num_targets = 1;
    assert_eq!(wide_targets_of(&one, view), None, "one target: the one-target shadow's");
    assert!(wide_any(&one), "which still takes it");

    let mut mixed = d;
    mixed.targets[2].format = GLES_SURF_RGBA8888;
    assert_eq!(wide_targets_of(&mixed, view), None, "a 32-bit target beside them keeps the linear path");
    assert!(!wide_any(&mixed));

    let no_view = |slot: u8| if slot == 2 { None } else { view(slot) };
    assert_eq!(wide_targets_of(&d, no_view), None, "attachment 2 has no CPU view");

    let mut gap = d;
    gap.targets[1].gpu = 0;
    let w = wide_targets_of(&gap, view).expect("a GL_NONE draw buffer between them is no target");
    assert_eq!((w.n, w.t[1].gpu), (2, mems[2].gpu), "the targets after it move down");
    free(&mut mems);
}

/// The 24 cases' draw: three RGBA16F targets, each rendered TILED into a
/// page-aligned shadow of its own, never into its linear storage. RT0 through
/// PE_PIPE_COLOR_ADDR and PE_COLOR_STRIDE, RT1 and RT2 through the 8-RT
/// bank, every stride a row of the 16-padded width, the PE format
/// A16B16G16R16F and SINGLE_BUFFER(2).
#[test]
fn three_rgba16f_targets_are_rendered_tiled_into_their_own_shadows() {
    let rig = Rig::new();
    let (mut mems, d, _fs) = wide_mrt(&rig, 3, GLES_SURF_RGBA16F, true);
    let s = rig.submit(&d);
    let rt0 = u64::from(s.reg(pe_pipe_color_addr(0)));
    assert_eq!(s.reg(pe_pipe_color_addr(1)) as u64, rt0);
    assert_eq!(s.reg(GC_PE_COLOR_STRIDE), SHADOW_STRIDE);
    assert_eq!((s.reg(GC_PE_COLOR_FORMAT) >> 24) & 0x7f, GC_PE_FORMAT_A16B16G16R16F);
    assert_eq!(single_buffer_of(&s), 2, "SINGLE_BUFFER(2): every target tiled");
    let f = s.reg(GC_PE_COLOR_FORMAT);
    assert_eq!(f & (GC_PE_COLOR_FORMAT_SUPER_TILED | GC_PE_COLOR_FORMAT_SUPER_TILED_NEW), 0, "TILED, not supertiled");
    let mut addrs = vec![rt0];
    for rt in 1..3u32 {
        addrs.push(u64::from(s.reg(pe_rt_addr_8_pipe(rt, 0))));
        assert_eq!(
            s.reg(pe_rt_config_8(rt)),
            rt_config_stride(SHADOW_STRIDE) | rt_config_format(GC_PE_FORMAT_A16B16G16R16F),
            "RT{rt}: the shadow's stride, the 64-bit float format"
        );
    }
    for (k, a) in addrs.iter().enumerate() {
        assert_eq!(a & 0xfff, 0, "target {k}'s shadow starts on a page: 0x{a:08x}");
        assert!(mems.iter().all(|m| m.gpu != *a), "target {k}: the PE never writes a linear storage");
    }
    addrs.sort_unstable();
    addrs.dedup();
    assert_eq!(addrs.len(), 3, "one shadow a target");
    assert_eq!(gc_finish(), 0);
    free(&mut mems);
}

/// Each shadow is tiled from its storage before the draw and written back
/// into it, and only it, when the kick completes: the storages are cleared
/// in between, so only the write-back restores them, every pixel of every
/// target at its own place.
#[test]
fn three_rgba16f_targets_come_back_into_their_own_storages() {
    let rig = Rig::new();
    let (mut mems, d, _fs) = wide_mrt(&rig, 3, GLES_SURF_RGBA16F, true);
    let want: Vec<Vec<u64>> = mems.iter().map(pixels).collect();
    let (tiled0, untiled0) = wt::counts();
    rig.submit(&d);
    for m in &mems {
        // SAFETY: the storage's BYTES.
        unsafe { core::ptr::write_bytes(m.cpu as *mut u8, 0, BYTES) };
    }
    assert_eq!(gc_finish(), 0);
    for (k, m) in mems.iter().enumerate() {
        assert_eq!(pixels(m), want[k], "target {k} written back into its own storage");
    }
    let (tiled, untiled) = wt::counts();
    assert_eq!((tiled - tiled0, untiled - untiled0), (3 * BYTES as u64, 3 * BYTES as u64));
    free(&mut mems);
}

/// RG16F is drawn as RGBA16F (the GL core keeps four halves of it,
/// `fb_target_n`): the `rg16f` array cases take the same three shadows. The
/// 64-bit integer targets (RGBA16I/UI, RG32I/UI), refused beside another
/// draw buffer because they kept the linear path, are drawn the same way.
#[test]
fn rg16f_and_64_bit_integer_arrays_take_the_same_shadows() {
    for (format, pe) in [
        (GLES_SURF_RGBA16F, GC_PE_FORMAT_A16B16G16R16F),
        (GLES_SURF_RGBA16I, GC_PE_FORMAT_A16B16G16R16I),
        (GLES_SURF_RG32UI, GC_PE_FORMAT_G32R32F),
    ] {
        let rig = Rig::new();
        let (mut mems, d, _fs) = wide_mrt(&rig, 3, format, true);
        let s = rig.submit(&d);
        assert_eq!(single_buffer_of(&s), 2, "format {format}");
        for rt in 1..3u32 {
            assert_eq!(s.reg(pe_rt_config_8(rt)), rt_config_stride(SHADOW_STRIDE) | rt_config_format(pe));
            assert_eq!(s.reg(pe_rt_addr_8_pipe(rt, 0)) & 0xfff, 0);
        }
        assert_eq!(gc_finish(), 0);
        free(&mut mems);
    }
}

/// Without a CPU view of attachments 1.. -- what the GL core hands over
/// today -- the draw is what it was at 9bc82915: every target linear,
/// SINGLE_BUFFER(1), the bank naming the storages; and a 64-bit integer
/// array is refused by name.
#[test]
fn without_cpu_views_several_64_bit_targets_keep_the_linear_path() {
    let rig = Rig::new();
    let (mut mems, d, _fs) = wide_mrt(&rig, 3, GLES_SURF_RGBA16F, false);
    assert_eq!(wide_targets(&d), None);
    let s = rig.submit(&d);
    assert_eq!(u64::from(s.reg(pe_pipe_color_addr(0))), mems[0].gpu);
    assert_eq!(single_buffer_of(&s), 1);
    for rt in 1..3usize {
        assert_eq!(u64::from(s.reg(pe_rt_addr_8_pipe(rt as u32, 0))), mems[rt].gpu);
        assert_eq!(
            s.reg(pe_rt_config_8(rt as u32)),
            rt_config_stride(64) | rt_config_format(GC_PE_FORMAT_A16B16G16R16F)
        );
    }
    assert_eq!(gc_finish(), 0);
    free(&mut mems);

    let (mut mems, d, _fs) = wide_mrt(&rig, 2, GLES_SURF_RGBA16UI, false);
    // SAFETY: `d` is a live draw for the length of the call.
    assert_eq!(unsafe { gc_draw(&d) }, -crate::util::ENOTSUP);
    free(&mut mems);
}
