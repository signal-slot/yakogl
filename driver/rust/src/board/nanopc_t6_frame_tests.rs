// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests for the generated Z4 frame (nanopc_t6_frame.rs): sizes,
//! alignment, the VA map, the stream's shape and the frame the submit step
//! builds from it. Feature set: `--features rk3588,mali` (also built under
//! plain `cargo test`).
//!
//! The frame file is generated (tools/mali/panthor/tests/t6_frame_blob.rs
//! compares its bytes), so these tests live here and not in a `#[cfg(test)]`
//! module of it. The file is const data: it reads no register, so the state
//! U-Boot leaves (VOP2 not scanning out) and the state Linux leaves give the
//! same values. [`the_frame_names_no_register_state`] pins that: it is a
//! pure `const` item set, evaluated the same cold or warm. Addresses are
//! restated here from driver/docs/nanopc-t6-mali.md ("Submit (Z4)"), not
//! taken from the module, so a wrong constant cannot agree with itself.

use super::nanopc_t6_frame as f;
use super::nanopc_t6_submit as sub;

const PAGE: u64 = 0x1000;

fn arenas() -> [(&'static str, u64, usize, &'static [u8]); 5] {
    [
        ("ctx", f::CTX_VA, f::CTX_SIZE, &f::CTX),
        ("desc", f::DESC_VA, f::DESC_SIZE, &f::DESC),
        ("code", f::CODE_VA, f::CODE_SIZE, &f::CODE),
        ("spds", f::SPDS_VA, f::SPDS_SIZE, &f::SPDS),
        ("data", f::DATA_VA, f::DATA_SIZE, &f::DATA),
    ]
}

#[test]
fn the_va_map_is_the_documented_one() {
    assert_eq!(f::CTX_VA, 0x1_0200_0000);
    assert_eq!(f::DESC_VA, 0x1_0201_0000);
    assert_eq!(f::CODE_VA, 0x1_0202_0000);
    assert_eq!(f::SPDS_VA, 0x1_0203_0000);
    assert_eq!(f::DATA_VA, 0x1_0204_0000);
    assert_eq!(f::GEOMETRY_BUFFER_VA, 0x1_0210_0000);
    assert_eq!(f::TARGET_VA, 0x1_0300_0000);
    assert_eq!(f::HEAP_FIRST_CHUNK_VA, 0x8000_00a0_0000);
    assert!(f::DRAW);
}

#[test]
fn sizes_and_used_lengths() {
    assert_eq!(f::CTX_SIZE, 0x1000);
    assert_eq!(f::DESC_SIZE, 0x4000);
    assert_eq!(f::CODE_SIZE, 0x1000);
    assert_eq!(f::SPDS_SIZE, 0x1000);
    assert_eq!(f::DATA_SIZE, 0x1000);
    assert_eq!(f::GEOMETRY_BUFFER_SIZE, 0x1_0000);
    assert_eq!(f::HEAP_CHUNK_SIZE, 0x20_0000);
    assert_eq!((f::CTX.len(), f::DESC.len(), f::CODE.len(), f::SPDS.len(), f::DATA.len()), (832, 1008, 256, 64, 48));
    for (name, _, size, bytes) in arenas() {
        assert!(!bytes.is_empty() && bytes.len() <= size, "{name}");
        assert_eq!(bytes.len() % 16, 0, "{name}: used length is whole 16-byte units");
        assert_eq!(size % PAGE as usize, 0, "{name}: memory is whole pages");
    }
    assert_eq!(f::WORDS.len(), 51);
}

#[test]
fn alignment_of_every_address() {
    for (name, va, _, _) in arenas() {
        assert_eq!(va % 0x1_0000, 0, "{name}: arenas start on a 64 KiB boundary");
    }
    assert_eq!(f::GEOMETRY_BUFFER_VA % 0x1_0000, 0);
    assert_eq!(f::TARGET_VA % 0x1_0000, 0);
    assert_eq!(f::HEAP_FIRST_CHUNK_VA % u64::from(f::HEAP_CHUNK_SIZE), 0, "the chunk is aligned to its size");
    assert!(f::HEAP_CHUNK_SIZE.is_power_of_two());
    assert!(f::GEOMETRY_BUFFER_SIZE.is_power_of_two());
}

#[test]
fn spans_are_clear_of_each_other_and_of_the_firmware_vas() {
    let mut spans: Vec<(u64, u64, &str)> = arenas().iter().map(|&(n, va, size, _)| (va, size as u64, n)).collect();
    spans.push((f::GEOMETRY_BUFFER_VA, u64::from(f::GEOMETRY_BUFFER_SIZE), "geometry"));
    spans.push((f::TARGET_VA, 64 * 256, "target"));
    spans.push((f::HEAP_FIRST_CHUNK_VA, u64::from(f::HEAP_CHUNK_SIZE), "heap"));
    spans.sort();
    // Z3's firmware (AS0) ends at 0x0410_0000; the frame lives in the group VM above it.
    assert!(spans[0].0 >= 0x0410_0000, "{spans:x?}");
    for p in spans.windows(2) {
        assert!(p[0].0 + p[0].1 <= p[1].0, "{} overlaps {}", p[0].2, p[1].2);
    }
    // 48-bit VA space.
    assert!(spans.iter().all(|s| s.0 + s.1 <= 1 << 48));
}

#[test]
fn the_fbd_pointer_names_the_desc_arena() {
    // RUN_FRAGMENT's pointer: the FBD address with tag bits in the low 6 bits.
    assert_eq!(f::FBD_POINTER & 0x3f, 0x01);
    let fbd = f::FBD_POINTER & !0x3f;
    assert_eq!(fbd % 64, 0);
    assert!((f::DESC_VA..f::DESC_VA + f::DESC.len() as u64).contains(&fbd));
    assert_eq!(fbd, f::DESC_VA);
    // the same pointer is the low half of the stream word that loads it
    let ptr_words: Vec<u64> = f::WORDS.iter().copied().filter(|w| w & 0xffff_ffff_ffff == f::FBD_POINTER).collect();
    assert_eq!(ptr_words.len(), 1, "one MOV48 loads the FBD pointer");
    assert_eq!(ptr_words[0] >> 56, 0x01, "opcode 1 = MOVE48");
}

#[test]
fn the_tiler_numbers() {
    assert_eq!(f::TILER_MAX_LEVELS, 8);
    assert_eq!(f::USER_REGS, 92);
    // every register the stream names (bits 48-55 of a MOVE48/MOVE32 word,
    // opcodes 1 and 2) is inside the user registers
    for w in f::WORDS {
        if matches!(w >> 56, 1 | 2) {
            assert!(((w >> 48) & 0xff) < u64::from(f::USER_REGS), "{w:#018x}");
        }
    }
}

#[test]
fn the_stream_does_not_start_with_the_queue_init() {
    // QUEUE_INIT is MOVE48 d72 <heap ctx>, HEAP_SET d72 (opcode 0x30): the
    // frame's own words come after it, and none of them is HEAP_SET.
    assert!(f::WORDS.iter().all(|w| w >> 56 != 0x30));
    assert_ne!(f::WORDS[0], sub::QUEUE_INIT[0]);
    // the first word's opcode, as generated
    assert_eq!(f::WORDS[0] >> 56, 0x22);
    // the stream ends on opcode 3 (a wait), not a branch
    assert_eq!(f::WORDS[f::WORDS.len() - 1] >> 56, 0x03);
}

#[test]
fn the_geometry_data_is_the_triangle_vertices() {
    // data arena: four f32 x/y/z/w groups of the clip-space triangles (-1, -1, 0, ...)
    let floats: Vec<f32> = f::DATA.chunks(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    assert_eq!(floats.len(), 12);
    assert!(floats.iter().all(|v| v.is_finite() && v.abs() <= 1.0), "{floats:?}");
    assert_eq!(&floats[..2], &[-1.0, -1.0]);
}

#[test]
fn the_submit_frame_is_built_from_the_file() {
    let g = sub::GENERATED;
    assert_eq!(g.heap_chunk_va, f::HEAP_FIRST_CHUNK_VA);
    assert_eq!(g.heap_chunk_size, f::HEAP_CHUNK_SIZE);
    assert_eq!(g.target_va, f::TARGET_VA);
    assert_eq!((g.width, g.height), (64, 64));
    assert_eq!(g.target_bytes, 64 * 64 * 4);
    // 64 pixels of 4 bytes per row is 256, the row stride the file says
    assert_eq!(g.width * 4, 256);
    assert_eq!(g.blocks.len(), 5);
    for ((va, bytes), (name, want_va, _, want)) in g.blocks.iter().zip(arenas()) {
        assert_eq!(*va, want_va, "{name}");
        assert_eq!(*bytes, want, "{name}");
    }
    // the stream is the queue-init words, then the file's, unchanged
    assert_eq!(g.cs.len(), sub::QUEUE_INIT.len() + f::WORDS.len());
    assert_eq!(&g.cs[..sub::QUEUE_INIT.len()], &sub::QUEUE_INIT);
    assert_eq!(&g.cs[sub::QUEUE_INIT.len()..], &f::WORDS);
    assert_eq!(g.digest, sub::digest(g.blocks, g.cs_va, g.cs));
}

#[test]
fn the_group_vm_backs_the_whole_frame() {
    // the arenas' region starts at ctx and covers up to data's memory end
    let arenas_region = sub::REGIONS[sub::REGION_ARENAS];
    assert_eq!(arenas_region.va, f::CTX_VA);
    assert!(arenas_region.va + arenas_region.size >= f::DATA_VA + f::DATA_SIZE as u64);
    assert!(arenas_region.va + arenas_region.size >= sub::CS_VA + (sub::CS.len() * 8) as u64);
    let g = sub::REGIONS[sub::REGION_GEOMETRY];
    assert_eq!((g.va, g.size), (f::GEOMETRY_BUFFER_VA, u64::from(f::GEOMETRY_BUFFER_SIZE)));
    let t = sub::REGIONS[sub::REGION_TARGET];
    assert_eq!(t.va, f::TARGET_VA);
    assert!(t.size >= u64::from(sub::TARGET_BYTES));
    assert_eq!(sub::REGIONS[sub::REGION_HEAP_CHUNK].size, u64::from(f::HEAP_CHUNK_SIZE));
    // the file's own frame passes the submit step's check when each region is backed
    // (check_frame reads only the sizes; the addresses are never dereferenced)
    let backing: Vec<sub::GroupBacking> = sub::REGIONS
        .iter()
        .enumerate()
        .map(|(i, r)| unsafe { sub::GroupBacking::new(0x4000_0000 + i as u64 * 0x1000_0000, 0x1000, r.size) })
        .collect();
    assert_eq!(sub::check_frame(&sub::GENERATED, &backing), Ok(()));
}

#[test]
fn the_frame_names_no_register_state() {
    // Every item of the file is a const: nothing is read at run time, so the
    // cold display state U-Boot leaves (VOP2 not scanning out, no PD_VOP) and
    // the one Linux leaves yield the same frame. Evaluating twice in a const
    // context is the pin: it compiles only while the file is const data.
    const A: usize = f::CTX.len() + f::DESC.len() + f::CODE.len() + f::SPDS.len() + f::DATA.len() + f::WORDS.len();
    const B: usize = 832 + 1008 + 256 + 64 + 48 + 51;
    assert_eq!(A, B);
}
