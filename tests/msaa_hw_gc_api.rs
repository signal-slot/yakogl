// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The public `msaa_hw::gc` words driven with `msaa_store::MsaaStore` the
//! way `gc-msaa-fbo` will drive them (docs/notes/es3-msaa-renderbuffers.md
//! (c3), (f3); docs/notes/gc-msaa-addresses.md section 2; gc-msaa-record.md).
//!
//! Run with:
//!   cargo test --test msaa_hw_gc_api --no-default-features --features host-test,gpu-vivante
//!
//! (`msaa_hw` and `msaa_store` are unconditional modules; the library itself
//! needs one back end feature to compile, here the GC7000UL's.)

use yakogl::backend::{GLES_SURF_RGB565, GLES_SURF_RGBA8888, GLES_SURF_SRGB8_A8, GLES_SURF_XRGB8888};
use yakogl::gl::consts::*;
use yakogl::msaa_hw::gc::*;
use yakogl::msaa_store::{MsaaStore, SampleSize, MSAA_SLOTS};

const SIZES: [(u32, u32); 4] = [(1, 1), (64, 64), (65, 33), (1920, 1080)];

/// A recording allocator: handles count up from 1.
#[derive(Default)]
struct Heap {
    next: u32,
    asked: Vec<usize>,
    freed: Vec<u32>,
}

fn get(t: &mut MsaaStore<u32>, h: &mut Heap, id: u64, gen: u32, size: SampleSize) -> Option<u32> {
    let (asked, next, freed) = (&mut h.asked, &mut h.next, &mut h.freed);
    t.get_or_alloc(
        id,
        gen,
        size,
        |b| {
            asked.push(b);
            *next += 1;
            Some(*next)
        },
        |x| freed.push(x),
    )
    .map(|s| s.handle)
}

#[test]
fn scratch_covers_four_samples_and_store_asks_for_them() {
    for (w, h) in SIZES {
        for bpp in [2u32, 4] {
            let (p, r, b) = (scratch_pitch(w, bpp), scratch_rows(h), scratch_bytes(w, h, bpp));
            assert_eq!(b, p * r, "{w}x{h}x{bpp}");
            assert_eq!(p % (64 * bpp), 0, "pitch is a supertile of samples");
            assert_eq!(r % 64, 0);
            let packed = w as u64 * h as u64 * bpp as u64 * 4;
            assert!(b as u64 >= packed, "{w}x{h}x{bpp}: {b} < {packed}");
            assert!(p >= 2 * w * bpp && r >= 2 * h);

            let s = sample_size(w, h, bpp);
            assert_eq!(s, SampleSize { w, h, bytes_per_sample: bpp, samples: 4 });
            let (mut t, mut heap) = (MsaaStore::<u32>::new(), Heap::default());
            assert!(get(&mut t, &mut heap, 1, 1, s).is_some());
            // The store asks for the packed samples; the back end's alloc
            // closure pads that up to scratch_bytes itself.
            assert_eq!(heap.asked, [packed as usize], "{w}x{h}x{bpp}");
            assert!(heap.asked[0] <= b as usize);
        }
    }
}

const COLOUR_4X: [(u32, u32); 8] = [
    (GL_RGBA8, RS_FORMAT_A8R8G8B8),
    (GL_RGB8, RS_FORMAT_A8R8G8B8),
    (GL_R8, RS_FORMAT_A8R8G8B8),
    (GL_RG8, RS_FORMAT_A8R8G8B8),
    (GL_SRGB8_ALPHA8, RS_FORMAT_A8R8G8B8),
    (GL_RGB565, RS_FORMAT_R5G6B5),
    (GL_RGBA4, RS_FORMAT_A4R4G4B4),
    (GL_RGB5_A1, RS_FORMAT_A1R5G5B5),
];

#[test]
fn renderable_formats_resolve_and_excluded_ones_do_not() {
    // Note (c3) "Formats" and (f3): the RGBA8 group, sRGB, and the 16-bit rows.
    for (f, rs) in COLOUR_4X {
        assert_eq!(rs_resolve_format(f), Some(rs), "0x{f:x}");
        assert!(rs < 0x20, "fits the 5-bit RS format field");
    }
    for f in [GL_DEPTH_COMPONENT16, GL_DEPTH_COMPONENT24, GL_DEPTH24_STENCIL8, GL_STENCIL_INDEX8] {
        assert_eq!(rs_resolve_format(f), Some(RS_FORMAT_D32), "0x{f:x}");
    }
    // Excluded by (c3)/(f3): RGB10_A2 (no RS row), floats, integers, unsized.
    for f in [
        GL_RGB10_A2,
        GL_R16F,
        GL_RG16F,
        GL_RGBA16F,
        GL_R32F,
        GL_RGBA32F,
        GL_R11F_G11F_B10F,
        GL_RGBA8I,
        GL_RGBA8UI,
        GL_R32UI,
        GL_RGB10_A2UI,
        GL_RGBA,
        GL_RGB,
    ] {
        assert_eq!(rs_resolve_format(f), None, "0x{f:x}");
    }
    // By stored surface: the 16-bit surface is never A4R4G4B4 by size alone.
    assert_eq!(rs_resolve_format_of_surface(GLES_SURF_RGBA8888), Some(RS_FORMAT_A8R8G8B8));
    assert_eq!(rs_resolve_format_of_surface(GLES_SURF_SRGB8_A8), Some(RS_FORMAT_A8R8G8B8));
    assert_eq!(rs_resolve_format_of_surface(GLES_SURF_XRGB8888), Some(RS_FORMAT_X8R8G8B8));
    assert_eq!(rs_resolve_format_of_surface(GLES_SURF_RGB565), Some(RS_FORMAT_R5G6B5));
    assert_eq!(rs_resolve_format_of_surface(0xff), None);
}

/// The RS words of one resolve of case 16's 64x64 RGBA8 4x target, against
/// the board's record: docs/notes/gc-msaa-addresses.md lines 75-78 (the
/// register table, "ours" column, same? yes) and gc-msaa-record.md line 116
/// (`x4 scratch: 65536 bytes`).
#[test]
fn rs_words_match_the_recorded_4x_frame() {
    let (w, h, bpp) = (64, 64, 4);
    assert!(rs_resolvable(w, h));
    let fmt = rs_resolve_format(GL_RGBA8).unwrap();
    assert_eq!(rs_config_resolve(fmt), 0x0000_06e6, "addresses.md:75 RS_CONFIG");
    let pitch = scratch_pitch(w, bpp);
    assert_eq!(pitch, 0x200, "addresses.md section 1: PE_COLOR_STRIDE 0x200");
    assert_eq!(rs_source_stride_resolve(pitch), 0x8800_0800, "addresses.md:76 RS_SOURCE_STRIDE");
    assert_eq!(rs_window_resolve(w, h), 0x0080_0080, "addresses.md:78 RS_WINDOW_SIZE");
    assert_eq!(scratch_bytes(w, h, bpp), 65536, "gc-msaa-record.md:116 scratch 65536 bytes");
    assert_eq!(
        rs_config_resolve(fmt),
        fmt | fmt << 8 | RS_CONFIG_SOURCE_TILED | RS_CONFIG_DOWNSAMPLE_X | RS_CONFIG_DOWNSAMPLE_Y
    );
    assert_eq!(rs_source_stride_resolve(pitch) & RS_SOURCE_STRIDE_TILING, RS_SOURCE_STRIDE_TILING);
    // Arm 19 state: UNK24 = 5 and SINGLE_BUFFER 2 for a 32-bit target
    // (addresses.md PE_LOGIC_OP rows).
    assert_eq!(pe_logic_op_4x(4), 0x0500_0200);
    assert_eq!(pe_logic_op_4x(4) & !PE_LOGIC_OP_4X_MASK, 0);
    // Extra PS input and temporary: varyings + 2 and MAX2(temps + 1, varyings + 2).
    assert_eq!(ps_input_count_4x(3), 5);
    assert_eq!(ps_temps_4x(1, 3), 5);
    assert_eq!(ps_temps_4x(9, 3), 10);
    assert_eq!(depth_extent_4x(w, h), (128, 128));
}

#[test]
fn store_slot_reuse_generation_and_eviction_with_gc_sizes() {
    const { assert!(MSAA_SLOTS >= 2, "(e)2: at least two slots") };
    let a = sample_size(64, 64, 4);
    let b = sample_size(128, 128, 4);
    let c = sample_size(64, 64, 2);
    let (mut t, mut h) = (MsaaStore::<u32>::new(), Heap::default());

    // First draw: no samples yet, so the back end must not load them.
    assert_eq!(get(&mut t, &mut h, 10, 1, a), Some(1));
    assert!(!t.get(10).unwrap().needs_load());
    // Job end: resolved, slot stored; the next job reuses it and loads.
    t.get(10).unwrap().mark_stored();
    assert_eq!(get(&mut t, &mut h, 10, 1, a), Some(1), "reuse: same handle");
    assert_eq!(h.asked.len(), 1);
    assert!(t.get(10).unwrap().needs_load());
    // A whole clear drops the validity.
    t.get(10).unwrap().mark_cleared();
    assert!(!t.get(10).unwrap().needs_load());
    t.get(10).unwrap().mark_stored();

    // Generation change (respecified renderbuffer): old scratch freed, fresh slot.
    assert_eq!(get(&mut t, &mut h, 10, 2, a), Some(2));
    assert_eq!(h.freed, [1]);
    assert!(!t.get(10).unwrap().needs_load());
    // A size change at the same gen behaves the same.
    assert_eq!(get(&mut t, &mut h, 10, 2, c), Some(3));
    assert_eq!(h.freed, [1, 2]);
    assert_eq!(h.asked, [64 * 64 * 4 * 4, 64 * 64 * 4 * 4, 64 * 64 * 2 * 4]);

    // Eviction: fill the slots, touch the first, add one more: the LRU goes.
    let mut t = MsaaStore::<u32>::new();
    let mut h = Heap::default();
    for id in 1..=MSAA_SLOTS as u64 {
        get(&mut t, &mut h, id, 1, b);
        t.get(id).unwrap().mark_stored();
    }
    get(&mut t, &mut h, 1, 1, b); // id 1 is now the most recent
    assert_eq!(t.len(), MSAA_SLOTS);
    let extra = MSAA_SLOTS as u64 + 1;
    get(&mut t, &mut h, extra, 1, b);
    assert_eq!(t.len(), MSAA_SLOTS);
    assert!(t.get(1).is_some(), "recently used survives");
    assert!(t.get(2).is_none(), "least recently used evicted, its scratch freed");
    assert_eq!(h.freed, [2]);
    // An evicted target comes back with nothing to load (refuse or clear).
    get(&mut t, &mut h, 2, 1, b);
    assert!(!t.get(2).unwrap().needs_load());
    // Deleting a renderbuffer frees its scratch; id 0 is never stored.
    t.gone(2, |x| h.freed.push(x));
    assert!(t.get(2).is_none());
    assert!(get(&mut t, &mut h, 0, 1, b).is_none());
    // Teardown frees everything.
    let live = t.len();
    let before = h.freed.len();
    t.clear(|x| h.freed.push(x));
    assert_eq!(h.freed.len(), before + live);
    assert!(t.is_empty());
}
