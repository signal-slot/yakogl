// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder against the board: the frame Mesa 25.0.7 drew on the
//! NanoPC-T6, built again from the same inputs with every arena based at
//! the trace's own addresses, compared byte for byte and word for word with
//! the dump (tools/mali/board-dump/tri-frame-2026-10-18.txt, the linear,
//! no-CRC run; every block the builder writes is in that dump, so the AFBC
//! run is not needed for any of them).
//!
//! Nothing is "close enough". Every place where our bytes may differ from
//! the dump's is listed in [`expected_differences`] with its cause, and the
//! test first checks that the dump holds there what the cause says (the
//! GPU's value, Mesa's stack, the moved block's old address), then replaces
//! it with what we must write, then requires every byte equal. The command
//! stream is compared the same way against the raw CS buffer, which also
//! holds the one word the printed trace skips.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::tagged;
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const FRAME: &str = include_str!("../../tools/mali/board-dump/tri-frame-2026-10-18.txt");

/* the trace's buffers (the "Buffer: memory_<va>" headers of the excerpt) */
const DESC: u64 = 0x7fff_fffd_4000;
const CODE: u64 = 0x7fff_ffff_c000;
/* Mesa's vertex program writes the points/generic variant at ...d000 and
 * the triangle variant the draw uses at ...d020 (pan_cmdstream.c:3995-4034):
 * the builder writes the triangle one only, so its arena starts there */
const SPDS: u64 = 0x7fff_ffff_d020;
const HEAP_DESC: u64 = 0x7fff_ffff_b000;
const SAMPLES: u64 = 0x7fff_ffff_f000;
const CS: u64 = 0x7fff_fffc_c000;
const CS_WORDS: u64 = 58;
/* the trace's blocks after the ones Mesa adds and the builder does not
 * (the OOM context at +0x260, three incremental-rendering FBDs at +0x340,
 * +0x440, +0x540) */
const TRACE_TILER: u64 = DESC + 0x2c0;
const TRACE_VS_RES: u64 = DESC + 0x640;
const TRACE_FS_RES: u64 = DESC + 0x6c0;

/* the inputs, as the trace prints them */
/// The tiler heap's first chunk and chunk size (lines 418-426: "Size:
/// 2097152", "Base: 0x800000a00000").
const HEAP_CHUNK: u64 = 0x8000_00a0_0000;
const HEAP_CHUNK_SIZE: u32 = 2 << 20;
/// The geometry buffer (lines 438-439).
const GEOMETRY: u64 = 0x7fff_fffe_b000;
/// The render target (lines 771-774) and the vertex buffer (lines 116-123).
const TARGET: u64 = 0x7fff_fffe_4000;
const VERTICES: u64 = 0x7fff_ffec_c000;

fn mem() -> BTreeMap<u64, [u8; 16]> {
    trace::memory_of(FRAME, "linear")
}

/// The position shader's sysval block and push words: the viewport scale
/// and offset, and the five words the binary reads as u0..u4
/// (`FMA r0, x, u0, u1`, `FMA r1, y, u2, u3`, `MOV r2, u4`, the
/// disassembly the shaders README quotes): Mesa's bi_opt_reorder_push pairs
/// the two words one instruction reads into one 64-bit slot, scale.x with
/// offset.x and scale.y with offset.y, and puts the unpaired offset.z last
/// (Mesa 25.0.7, src/panfrost/compiler/bi_opt_push_ubo.c:103-133, 284-355).
/// The trace cannot tell scale from offset (both are 32.0 for a 64-pixel
/// viewport); the pairing is the compiler's rule, and the order is the one
/// the binary's arithmetic needs.
const VS_SYSVALS: [Sysval; 2] = [Sysval::ViewportScale, Sysval::ViewportOffset];
const VS_PUSH: [PushWord; 5] = [
    PushWord::Sysval { sysval: 0, component: 0 },
    PushWord::Sysval { sysval: 1, component: 0 },
    PushWord::Sysval { sysval: 0, component: 1 },
    PushWord::Sysval { sysval: 1, component: 1 },
    PushWord::Sysval { sysval: 1, component: 2 },
];

fn meta(stage: ShaderStage, preload: u32) -> ShaderMeta<'static> {
    let vertex = stage == ShaderStage::VERTEX;
    ShaderMeta {
        stage,
        /* "Register allocation: 32 Per Thread" (lines 327, 362) */
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        preload,
        sysvals: if vertex { &VS_SYSVALS } else { &[] },
        push: if vertex { &VS_PUSH } else { &[] },
        varyings: 0,
        varying_bytes: 0,
        secondary: None,
        no_psiz_offset: None,
        colour_outputs: if vertex { 0 } else { 1 },
        samples_texture: false,
        requires_helper_threads: false,
        can_discard: false,
        writes_depth: false,
        writes_stencil: false,
        writes_coverage: false,
        writes_global: false,
        ubo: &[],
    }
}

/// The arenas' memory and the stream's words of one built frame.
struct Built {
    desc: Vec<u8>,
    code: Vec<u8>,
    spds: Vec<u8>,
    heap: Vec<u8>,
    samples: Vec<u8>,
    words: Vec<u64>,
    frame: Frame,
}

/// Build the board's frame from the trace's inputs, every arena at the
/// trace's address of its first block.
fn build() -> Built {
    let m = mem();
    /* the two binaries, seven instructions each (lines 342-348, 377-383) */
    let vs_bin = trace::bytes(&m, CODE, 7 * 8);
    let fs_bin = trace::bytes(&m, CODE + 0x80, 7 * 8);
    build_with(&vs_bin, meta(ShaderStage::VERTEX, 0x3800), &fs_bin, meta(ShaderStage::FRAGMENT, 0x3000))
}

/// The board's frame with the two programs given, every arena where
/// [`build`] puts it.
fn build_with(vs_bin: &[u8], vs_meta: ShaderMeta, fs_bin: &[u8], fs_meta: ShaderMeta) -> Built {
    /* room for the two programs padded, and three SPDs (a varying one) */
    let (mut desc_m, mut code_m, mut spd_m) = (vec![0u8; 0x1000], vec![0u8; 0x400], vec![0u8; 0x60]);
    let (mut heap_m, mut samples_m) = (vec![0u8; 0x20], vec![0u8; 0x300]);
    let mut words = vec![0u64; 64];
    let frame;
    {
        let mut code = Arena::new("code", CODE, &mut code_m).expect("arena");
        let mut spds = Arena::new("spds", SPDS, &mut spd_m).expect("arena");
        let vs = upload_program(&mut code, &mut spds, vs_bin, vs_meta).expect("vs");
        let fs = upload_program(&mut code, &mut spds, fs_bin, fs_meta).expect("fs");
        let mut heap = Arena::new("heap", HEAP_DESC, &mut heap_m).expect("arena");
        let heap_desc = tiler_heap_descriptor(&mut heap, HEAP_CHUNK, HEAP_CHUNK_SIZE).expect("heap");
        let mut samples = Arena::new("samples", SAMPLES, &mut samples_m).expect("arena");
        let positions = sample_positions(&mut samples).expect("samples");
        let ctx = Context {
            heap: heap_desc.gpu,
            geometry_buffer: GEOMETRY,
            geometry_buffer_size: 65536,
            sample_positions: positions.gpu,
            tiler_max_levels: 8,
        };
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: VERTICES, size: 1 << 20 },
            offset: 0,
            stride: 8,
            components: 2,
            instance_divisor: 0,
        }];
        let input = FrameInput {
            zs: None,
            target: Target {
                gpu: TARGET,
                width: 64,
                height: 64,
                row_stride: 256,
                format: TargetFormat::Rgba8888,
                samples: 1,
            },
            /* glClearColor(0, 0, 1, 1) */
            clear_rgba: Some([0.0, 0.0, 1.0, 1.0]),
            draw: Some(DrawCall {
                zs: DepthStencilState::OFF,
                colour_mask: 0xf,
                front_cw: false,
                cull: Cull::None,
                textures: &[],
                vs_texture: None,
                occlusion_query: None,
                alpha_to_coverage: false,
                blend_shader: None,
                vs: &vs,
                fs: &fs,
                primitive: Topology::Triangles,
                first_vertex: 0,
                vertex_count: 6,
                instance_count: 1,
                indices: None,
                attributes: &attributes,
                viewport: Viewport { x: 0, y: 0, width: 64, height: 64, near: 0.0, far: 1.0 },
                scissor: None,
                line_width: 1.0,
                flip_y: false,
                blend_color: [0.0; 4],
                blend: None,
                polygon_offset: None,
                sample_mask: 0xf,
                vs_uniforms: &[],
                fs_uniforms: &[],
            }),
        };
        let mut desc = Arena::new("desc", DESC, &mut desc_m).expect("arena");
        /* 92 user registers, Mesa 25.0.7's (pan_csf.c:250-252) */
        let mut cs = Stream::new(&mut words, 92);
        frame = build_frame(&mut desc, &mut cs, &ctx, &input).expect("the board's frame builds");
        let n = cs.words().len();
        words.truncate(n);
    }
    Built { desc: desc_m, code: code_m, spds: spd_m, heap: heap_m, samples: samples_m, words, frame }
}

/// One place where our bytes are allowed to differ from the dump's: the
/// block, the byte offset in it, what the dump holds there and why, and
/// what we write instead.
struct Difference {
    block: &'static str,
    at: usize,
    dump: Vec<u8>,
    ours: Vec<u8>,
    cause: &'static str,
}

fn expected_differences(b: &Built) -> Vec<Difference> {
    let tiler = b.frame.layout.tiler.expect("a tiler context").gpu;
    vec![
        Difference {
            block: "Framebuffer",
            at: 0x38,
            dump: TRACE_TILER.to_le_bytes().to_vec(),
            ours: tiler.to_le_bytes().to_vec(),
            cause: "layout: the Tiler pointer names the tiler context, which comes earlier here: Mesa \
                    allocates the tiler OOM context and three incremental-rendering FBDs before it \
                    (pan_csf.c:1032-1051) and the builder does not",
        },
        Difference {
            block: "VS sysval block",
            at: 0x1c,
            dump: vec![0xc5, 0x33, 0x5c, 0xde],
            ours: vec![0; 4],
            cause: "Mesa writes x y z of each vec3 sysval (pan_cmdstream.c:989-1010) into a stack \
                    array (alloca, :1404-1409) and copies all 16 bytes: the fourth lane of the offset \
                    is what that stack held. The scale's fourth lane (+0x0c) is 0 in this dump by the \
                    same chance, so it is not listed",
        },
        Difference {
            block: "Tiler Context",
            at: 0,
            dump: 0x0002_8000_00bf_ff00u64.to_le_bytes().to_vec(),
            ours: vec![0; 8],
            cause: "GPU-written: the Polygon List (the dump is taken after the job ran); Mesa packs \
                    the descriptor without it (pan_csf.c:668-697)",
        },
        Difference {
            block: "Tiler Context",
            at: 0x60,
            dump: vec![1, 0, 0, 0],
            ours: vec![0; 4],
            cause: "GPU-written: Private State word 0, which Mesa does not pack (pan_csf.c:668-697)",
        },
        Difference {
            block: "Tiler Heap",
            at: 16,
            dump: 0x8000_00a0_04c0u64.to_le_bytes().to_vec(),
            ours: (HEAP_CHUNK + 64).to_le_bytes().to_vec(),
            cause: "GPU-written: Bottom; Mesa writes base + 64 (pan_csf.c:1440)",
        },
        Difference {
            block: "Tiler Heap",
            at: 24,
            dump: 0x8000_00bf_fe00u64.to_le_bytes().to_vec(),
            ours: (HEAP_CHUNK + u64::from(HEAP_CHUNK_SIZE)).to_le_bytes().to_vec(),
            cause: "GPU-written: Top; Mesa writes base + size (pan_csf.c:1441)",
        },
    ]
}

/// Every block the builder writes: its name, our address and bytes, and
/// the dump's address of the same block.
fn blocks(b: &Built) -> Vec<(&'static str, u64, Vec<u8>, u64)> {
    let l = b.frame.layout;
    let ours = |mem: &[u8], base: u64, va: u64, len: usize| {
        let at = (va - base) as usize;
        mem[at..at + len].to_vec()
    };
    let d = |blk: Option<super::arena::Block>, len: usize| {
        let blk = blk.expect("a block");
        assert_eq!(blk.len, len, "{blk:?}");
        (blk.gpu, ours(&b.desc, DESC, blk.gpu, len))
    };
    let fbd = l.fbd.expect("an fbd").gpu;
    let mut v = vec![
        ("Framebuffer", fbd, ours(&b.desc, DESC, fbd, 128), DESC),
        ("Render Target", fbd + 128, ours(&b.desc, DESC, fbd + 128, 64), DESC + 0x80),
        ("Framebuffer ZS/CRC room (unused)", fbd + 192, ours(&b.desc, DESC, fbd + 192, 64), DESC + 0xc0),
    ];
    for (name, blk, len, trace_va) in [
        ("Local Storage", l.tls, 32, DESC + 0x100),
        ("Depth/Stencil", l.depth_stencil, 32, DESC + 0x120),
        ("Blend", l.blend, 16, DESC + 0x140),
        ("Attribute", l.attributes, 32, DESC + 0x160),
        ("Vertex Buffer", l.buffers, 32, DESC + 0x180),
        ("VS Sampler", l.vs_sampler, 32, DESC + 0x1a0),
        ("VS sysval block", l.vs_sysvals, 32, DESC + 0x1c0),
        ("VS UBO table", l.vs_ubos, 32, DESC + 0x1e0),
        ("VS push constants", l.vs_push, 24, DESC + 0x200),
        ("FS Sampler", l.fs_sampler, 32, DESC + 0x220),
        /* the fragment shader names no sysval: an empty block, so only its
         * place is compared (it sits between the sampler and the UBO table) */
        ("FS sysval block (empty)", l.fs_sysvals, 0, DESC + 0x240),
        ("FS UBO table", l.fs_ubos, 32, DESC + 0x240),
        ("Tiler Context", l.tiler, 128, TRACE_TILER),
        ("VS resource table", l.vs_resources, 112, TRACE_VS_RES),
        ("FS resource table", l.fs_resources, 112, TRACE_FS_RES),
    ] {
        let (va, bytes) = d(blk, len);
        v.push((name, va, bytes, trace_va));
    }
    v.push(("Tiler Heap", HEAP_DESC, b.heap[..32].to_vec(), HEAP_DESC));
    v.push(("Sample positions (single)", SAMPLES, b.samples[..256].to_vec(), SAMPLES));
    v.push(("VS Shader Program", SPDS, b.spds[..32].to_vec(), SPDS));
    v.push(("FS Shader Program", SPDS + 32, b.spds[32..64].to_vec(), SPDS + 32));
    v.push(("VS binary", CODE, b.code[..128].to_vec(), CODE));
    v.push(("FS binary", CODE + 128, b.code[128..256].to_vec(), CODE + 128));
    v
}

/// (a) Every descriptor byte the builder writes against the dump.
#[test]
fn the_frame_s_descriptors_are_the_board_s_bytes() {
    let b = build();
    let m = mem();
    let blocks = blocks(&b);
    let diffs = expected_differences(&b);
    let (mut equal_as_is, mut equal) = (0usize, 0usize);
    let mut failures = Vec::new();
    let mut used = vec![false; diffs.len()];
    for (name, our_va, ours, trace_va) in &blocks {
        let dump = trace::bytes(&m, *trace_va, ours.len());
        if &dump == ours {
            equal_as_is += 1;
        }
        let mut expected = dump.clone();
        for (k, diff) in diffs.iter().enumerate().filter(|(_, d)| d.block == *name) {
            used[k] = true;
            let span = diff.at..diff.at + diff.dump.len();
            assert_eq!(
                dump[span.clone()],
                diff.dump[..],
                "{name} +{:#x}: the dump is not what the cause says",
                diff.at
            );
            expected[span].copy_from_slice(&diff.ours);
        }
        if &expected == ours {
            equal += 1;
        } else {
            let at: Vec<usize> = (0..ours.len()).filter(|&i| expected[i] != ours[i]).collect();
            failures.push(format!(
                "{name} (ours {our_va:#x}, dump {trace_va:#x}): bytes {at:x?}\n  dump  {dump:02x?}\n  ours  {ours:02x?}"
            ));
        }
    }
    assert!(used.iter().all(|&u| u), "every listed difference names a compared block");
    eprintln!(
        "mali frame descriptors vs the board (linear dump, Mesa 25.0.7): {} compared, {} equal as dumped, \
         {} equal with the {} named differences, {} differing",
        blocks.len(),
        equal_as_is,
        equal,
        diffs.len(),
        failures.len()
    );
    for d in &diffs {
        eprintln!("  {} +{:#x}: {}", d.block, d.at, d.cause);
    }
    /* the empty block compares no byte: its length and place are the check */
    let fs_sysvals = b.frame.layout.fs_sysvals.expect("the fragment stage's sysval block");
    assert_eq!((fs_sysvals.len, fs_sysvals.gpu), (0, DESC + 0x240));
    assert_eq!(blocks.len(), 24, "the blocks this frame writes");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(equal_as_is, blocks.len() - 4, "four blocks hold a named difference");
}

/// What the dump holds where Mesa put the blocks the builder does not
/// build: checked, so that "not built" names a real block.
#[test]
fn the_blocks_not_built_are_where_the_dump_has_them() {
    let m = mem();
    let u64_at = |va: u64| u64::from_le_bytes(trace::bytes(&m, va, 8).try_into().expect("8 bytes"));
    /* Mesa's points/generic vertex SPD, the same as the triangle one */
    assert_eq!(trace::bytes(&m, SPDS - 0x20, 32), trace::bytes(&m, SPDS, 32));
    /* the OOM context (struct pan_csf_tiler_oom_ctx, packed, pan_csf.h:44-60):
     * counter 0, then three (CPU pointer, tagged FBD) pairs, then the
     * bounding box, the tiler context and the register save area */
    assert_eq!(trace::bytes(&m, DESC + 0x260, 4), [0; 4]);
    for (i, fbd) in [0x340u64, 0x440, 0x540].iter().enumerate() {
        assert_eq!(u64_at(DESC + 0x26c + 16 * i as u64), DESC + fbd + 1, "fbds[{i}].gpu");
        /* each is a framebuffer descriptor: its sample locations */
        assert_eq!(u64_at(DESC + fbd + 0x10), SAMPLES);
    }
    assert_eq!(u64_at(DESC + 0x29c), TRACE_TILER, "the OOM context's tiler_desc");
    /* the PAN_DBG_SYNC error-state word (pan_csf.c:336-346), 0 after the run */
    assert_eq!(u64_at(DESC + 0xa60), 0);
}

/// The raw CS words of the batch, in order.
fn raw_cs() -> Vec<u64> {
    let m = mem();
    (0..CS_WORDS).map(|k| u64::from_le_bytes(trace::bytes(&m, CS + 8 * k, 8).try_into().expect("8 bytes"))).collect()
}

fn move48(word: u64) -> (u32, u64) {
    match Instr::decode(word) {
        Ok(Instr::Move48(m)) => (m.destination, m.immediate),
        other => panic!("{word:#018x} is not a MOVE48: {other:?}"),
    }
}

/// (b) Every CS word against the raw batch buffer.
#[test]
fn the_frame_s_command_stream_is_the_board_s_words() {
    let b = build();
    let raw = raw_cs();
    let l = b.frame.layout;
    /* the words Mesa emits and the builder does not, each checked */
    let absent: [(usize, &str); 7] = [
        (2, "MOVE d76, the tiler OOM context (pan_csf.c:1049)"),
        (42, "LOAD_MULTIPLE r78, [d76]: the OOM counter (pan_csf.c:823)"),
        (43, "WAIT #1 after it"),
        (44, "BRANCH.le r78: skip the next move when no OOM happened (pan_csf.c:825)"),
        (45, "MOVE d40, the last incremental-rendering FBD (pan_csf.c:826)"),
        (53, "MOVE d90, the PAN_DBG_SYNC error-state word (pan_csf.c:336-343)"),
        (54, "STORE_STATE.ERROR_STATE [d90] (pan_csf.c:344-345)"),
    ];
    assert_eq!(move48(raw[2]), (76, DESC + 0x260));
    assert!(matches!(Instr::decode(raw[42]), Ok(Instr::LoadMultiple(i)) if i.base_register == 78 && i.address == 76));
    assert_eq!(raw[43], raw[50], "the same WAIT #1");
    assert!(matches!(Instr::decode(raw[44]), Ok(Instr::Branch(i)) if i.value == 78 && i.offset == 1));
    assert_eq!(move48(raw[45]), (40, DESC + 0x541));
    assert_eq!(move48(raw[53]), (90, DESC + 0xa60));
    assert!(
        matches!(Instr::decode(raw[54]), Ok(Instr::StoreState(i)) if i.address == 90 && i.state == CsState::ERROR_STATUS)
    );
    /* the words whose immediate is a block that moved: the dump's value
     * is the block's address in the trace (with the table count in the low
     * bits of an SRT), ours the block's address here */
    let tiler = l.tiler.expect("tiler").gpu;
    let moved: [(usize, u32, u64, u64); 4] = [
        (4, 0, TRACE_VS_RES | 7, l.vs_resources.expect("vs").gpu | 7),
        (8, 4, TRACE_FS_RES | 7, l.fs_resources.expect("fs").gpu | 7),
        (16, 40, TRACE_TILER, tiler),
        (48, 90, TRACE_TILER, tiler),
    ];
    let mut expected = Vec::new();
    let mut relocated = 0;
    for (k, &word) in raw.iter().enumerate() {
        if absent.iter().any(|&(i, _)| i == k) {
            continue;
        }
        if let Some(&(_, dst, was, now)) = moved.iter().find(|m| m.0 == k) {
            assert_eq!(move48(word), (dst, was), "raw word {k}");
            let again = Instr::Move48(CsMove48 { destination: dst, immediate: now, ..Default::default() });
            expected.push(again.encode().expect("encodes"));
            relocated += 1;
            continue;
        }
        expected.push(word);
    }
    let differ: Vec<String> = (0..expected.len().max(b.words.len()))
        .filter(|&i| expected.get(i) != b.words.get(i))
        .map(|i| format!("word {i}: expected {:x?}, ours {:x?}", expected.get(i), b.words.get(i)))
        .collect();
    let as_dumped = b.words.iter().filter(|w| raw.contains(w)).count();
    eprintln!(
        "mali frame CS vs the board (raw batch buffer, {} words): {} ours, {} equal as dumped, {} relocated, \
         {} of Mesa's not emitted, {} differing",
        raw.len(),
        b.words.len(),
        as_dumped,
        relocated,
        absent.len(),
        differ.len()
    );
    for (i, why) in absent {
        eprintln!("  not emitted: raw word {i}: {why}");
    }
    assert!(differ.is_empty(), "{}", differ.join("\n"));
    assert_eq!(b.words.len(), 51);
    assert_eq!(as_dumped, 47);
}

/// The queue-init stream against the trace's first two printed words
/// (`MOVE d72, #0x800000015000`, `HEAP_SET d72`), built from the heap
/// context address the trace's kernel returned, and its words decoded back
/// as HEAP_SET of that register holding that address. The trace's next three
/// words install the OOM handler, which is not built (`queue_init`).
#[test]
fn the_queue_init_stream_is_the_trace_s_heap_set() {
    let printed = trace::cs_words(FRAME, "linear");
    let heap_ctx = 0x8000_0001_5000;
    let mut words = [0u64; 4];
    let mut cs = Stream::new(&mut words, 92);
    queue_init(&mut cs, heap_ctx).expect("the init stream builds");
    let ours = cs.words().to_vec();
    assert_eq!(printed[0].mnemonic, "MOVE d72, #0x800000015000");
    assert_eq!(printed[1].mnemonic, "HEAP_SET d72");
    assert_eq!(ours, [printed[0].word, printed[1].word]);
    assert_eq!(move48(ours[0]), (72, heap_ctx));
    assert!(matches!(Instr::decode(ours[1]), Ok(Instr::HeapSet(i)) if i.address == 72));
    /* another address moves the immediate only */
    let mut words = [0u64; 4];
    let mut cs = Stream::new(&mut words, 92);
    queue_init(&mut cs, 0x8000_0002_3000).expect("builds");
    assert_eq!(move48(cs.words()[0]), (72, 0x8000_0002_3000));
    assert_eq!(cs.words()[1], printed[1].word);
    /* a stream whose user registers stop below d72 refuses, by name */
    let mut words = [0u64; 4];
    let mut cs = Stream::new(&mut words, 72);
    let e = queue_init(&mut cs, heap_ctx).unwrap_err();
    assert!(matches!(e, FrameError::Cs(super::cs::CsError::KernelRegister { reg: 73, .. })), "{e:?}");
}

/// A frame with only the clear: no tiler, no draw state, the FBD's Tiler
/// pointer 0 (Mesa's `batch->tiler_ctx` stays 0 without a draw). There is
/// no board trace of a clear-only frame; the words are the ones the draw
/// frame has around its fragment job, minus the tiler's.
#[test]
fn a_clear_only_frame_is_the_fragment_job_alone() {
    let mut desc_m = vec![0u8; 0x200];
    let mut words = [0u64; 32];
    let mut desc = Arena::new("desc", DESC, &mut desc_m).expect("arena");
    let mut cs = Stream::new(&mut words, 92);
    let ctx = Context {
        heap: HEAP_DESC,
        geometry_buffer: GEOMETRY,
        geometry_buffer_size: 65536,
        sample_positions: SAMPLES,
        tiler_max_levels: 8,
    };
    let input = FrameInput {
        zs: None,
        target: Target {
            gpu: TARGET,
            width: 64,
            height: 64,
            row_stride: 256,
            format: TargetFormat::Rgba8888,
            samples: 1,
        },
        clear_rgba: Some([0.0, 0.0, 1.0, 1.0]),
        draw: None,
    };
    let f = build_frame(&mut desc, &mut cs, &ctx, &input).expect("builds");
    assert!(f.layout.tiler.is_none());
    let names: Vec<&str> = cs.words().iter().map(|&w| Instr::decode(w).expect("decodes").name()).collect();
    assert_eq!(
        names,
        [
            "REQ_RESOURCE",
            "SET_SB_ENTRY",
            "MOVE48",
            "MOVE32",
            "MOVE32",
            "MOVE48",
            "MOVE32",
            "RUN_FRAGMENT",
            "WAIT",
            "WAIT",
            "MOVE32",
            "FLUSH_CACHE2",
            "WAIT"
        ]
    );
    /* the same render target as the draw frame's: only the FBD's Tiler differs */
    let b = build();
    assert_eq!(desc.used()[0x80..0xc0], b.desc[0x80..0xc0]);
    assert_eq!(desc.used()[0x38..0x40], [0; 8]);
}

/// Each draw the builder does not build is refused with its own line.
#[test]
fn each_draw_not_built_is_refused_by_name() {
    let m = mem();
    let bin = trace::bytes(&m, CODE, 56);
    let (mut code_m, mut spd_m) = (vec![0u8; 0x100], vec![0u8; 0x40]);
    let mut code = Arena::new("code", CODE, &mut code_m).expect("arena");
    let mut spds = Arena::new("spds", SPDS, &mut spd_m).expect("arena");
    let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX, 0x3800)).expect("vs");
    let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT, 0x3000)).expect("fs");
    /* a discarding shader is built (frame_discard_tests.rs); one that
     * writes depth is not */
    let mut fs_discard = fs;
    fs_discard.meta.writes_depth = true;
    let mut fs_two = fs;
    fs_two.meta.colour_outputs = 3;
    let mut vs_vary = vs;
    vs_vary.meta.varyings = 1;
    let bad_push = [PushWord::Sysval { sysval: 2, component: 0 }];
    let mut vs_push = vs;
    vs_push.meta.push = &bad_push;
    let past_uniforms = [PushWord::Uniform { dword: 0 }];
    let mut vs_uniform = vs;
    vs_uniform.meta.push = &past_uniforms;
    let past_halves = [PushWord::UniformHalves { lo: 0, hi: Some(4) }];
    let mut fs_halves = fs;
    fs_halves.meta.push = &past_halves;
    let four = [0u32; 4];
    let many = [PushWord::Sysval { sysval: 0, component: 0 }; 4 * 32 * 2 + 1];
    let mut vs_many = vs;
    vs_many.meta.push = &many;
    let many_sysvals = vec![Sysval::ViewportScale; 1025];
    let mut vs_sysvals = vs;
    vs_sysvals.meta.sysvals = &many_sysvals;
    let attributes = [VertexAttribute {
        buffer: VertexBuffer { gpu: VERTICES, size: 48 },
        offset: 0,
        stride: 8,
        components: 2,
        instance_divisor: 0,
    }];
    let five = [VertexAttribute { components: 5, ..attributes[0] }];
    let divisor_two = [VertexAttribute { instance_divisor: 2, ..attributes[0] }];
    let seventeen = [attributes[0]; 17];
    let draw = DrawCall {
        zs: DepthStencilState::OFF,
        colour_mask: 0xf,
        front_cw: false,
        cull: Cull::None,
        textures: &[],
        vs_texture: None,
        occlusion_query: None,
        alpha_to_coverage: false,
        blend_shader: None,
        vs: &vs,
        fs: &fs,
        primitive: Topology::Triangles,
        first_vertex: 0,
        vertex_count: 6,
        instance_count: 1,
        indices: None,
        attributes: &attributes,
        viewport: Viewport { x: 0, y: 0, width: 64, height: 64, near: 0.0, far: 1.0 },
        scissor: None,
        line_width: 1.0,
        flip_y: false,
        blend_color: [0.0; 4],
        blend: None,
        polygon_offset: None,
        sample_mask: 0xf,
        vs_uniforms: &[],
        fs_uniforms: &[],
    };
    let target =
        Target { gpu: TARGET, width: 64, height: 64, row_stride: 256, format: TargetFormat::Rgba8888, samples: 1 };
    fn with<'a>(target: Target, draw: DrawCall<'a>) -> FrameInput<'a> {
        FrameInput { zs: None, target, clear_rgba: Some([0.0; 4]), draw: Some(draw) }
    }
    let f = |draw| with(target, draw);
    let ctx = Context {
        heap: HEAP_DESC,
        geometry_buffer: GEOMETRY,
        geometry_buffer_size: 65536,
        sample_positions: SAMPLES,
        tiler_max_levels: 8,
    };
    let cases = [
        (FrameInput { zs: None, target, clear_rgba: None, draw: None }, NOT_BUILT_NO_CLEAR),
        (
            FrameInput { zs: None, target: Target { width: 0, ..target }, clear_rgba: Some([0.0; 4]), draw: None },
            NOT_BUILT_EMPTY_TARGET,
        ),
        (
            f(DrawCall {
                zs: DepthStencilState::OFF,
                colour_mask: 0xf,
                front_cw: false,
                cull: Cull::None,
                instance_count: 2,
                attributes: &divisor_two,
                ..draw
            }),
            NOT_BUILT_INSTANCE_DIVISOR,
        ),
        (
            f(DrawCall { zs: DepthStencilState::OFF, colour_mask: 0xf, front_cw: false, attributes: &five, ..draw }),
            NOT_BUILT_ATTRIBUTES,
        ),
        (
            f(DrawCall {
                zs: DepthStencilState::OFF,
                colour_mask: 0xf,
                front_cw: false,
                cull: Cull::None,
                textures: &[],
                vs_texture: None,
                attributes: &seventeen,
                ..draw
            }),
            NOT_BUILT_ATTRIBUTES,
        ),
        (
            f(DrawCall { zs: DepthStencilState::OFF, colour_mask: 0xf, front_cw: false, attributes: &[], ..draw }),
            NOT_BUILT_ATTRIBUTES,
        ),
        (
            f(DrawCall { zs: DepthStencilState::OFF, colour_mask: 0xf, front_cw: false, fs: &fs_discard, ..draw }),
            NOT_BUILT_FS_EFFECTS,
        ),
        (
            f(DrawCall { zs: DepthStencilState::OFF, colour_mask: 0xf, front_cw: false, fs: &fs_two, ..draw }),
            NOT_BUILT_FS_OUTPUTS,
        ),
        (
            f(DrawCall { zs: DepthStencilState::OFF, colour_mask: 0xf, front_cw: false, vs: &fs, ..draw }),
            NOT_BUILT_STAGE,
        ),
        (
            f(DrawCall { zs: DepthStencilState::OFF, colour_mask: 0xf, front_cw: false, vs: &vs_push, ..draw }),
            NOT_BUILT_PUSH,
        ),
        /* no uniform storage at all, and a high half past four dwords */
        (
            f(DrawCall { zs: DepthStencilState::OFF, colour_mask: 0xf, front_cw: false, vs: &vs_uniform, ..draw }),
            NOT_BUILT_PUSH_UNIFORM,
        ),
        (
            f(DrawCall {
                zs: DepthStencilState::OFF,
                colour_mask: 0xf,
                front_cw: false,
                cull: Cull::None,
                textures: &[],
                vs_texture: None,
                fs: &fs_halves,
                fs_uniforms: &four,
                ..draw
            }),
            NOT_BUILT_PUSH_UNIFORM,
        ),
        (
            f(DrawCall { zs: DepthStencilState::OFF, colour_mask: 0xf, front_cw: false, vs: &vs_many, ..draw }),
            NOT_BUILT_PUSH_COUNT,
        ),
        (
            f(DrawCall { zs: DepthStencilState::OFF, colour_mask: 0xf, front_cw: false, vs: &vs_sysvals, ..draw }),
            NOT_BUILT_SYSVALS,
        ),
    ];
    let mut seen = Vec::new();
    /* a tiler with other than the measured 8 levels refuses a draw, not a clear */
    let levels = Context { tiler_max_levels: 7, ..ctx };
    let draw_input = f(draw);
    let cases: Vec<_> =
        cases.iter().map(|(i, w)| (i, w, ctx)).chain([(&draw_input, &NOT_BUILT_TILER_LEVELS, levels)]).collect();
    for (input, why, ctx) in &cases {
        let mut desc_m = vec![0u8; 0x1000];
        let mut words = [0u64; 64];
        let mut desc = Arena::new("desc", DESC, &mut desc_m).expect("arena");
        let mut cs = Stream::new(&mut words, 92);
        let e = build_frame(&mut desc, &mut cs, ctx, input).expect_err(why);
        assert_eq!(e, FrameError::NotBuilt(why), "{why}");
        assert_eq!(cs.words().len(), 0, "{why}: refused before any word");
        if !seen.contains(why) {
            seen.push(why);
        }
    }
    for (i, a) in seen.iter().enumerate() {
        for b in &seen[i + 1..] {
            assert_ne!(a, b);
        }
    }
    assert_eq!(seen.len(), 12);
    /* a stage with varyings and no varying program is the record's defect */
    let input = f(DrawCall { zs: DepthStencilState::OFF, colour_mask: 0xf, front_cw: false, vs: &vs_vary, ..draw });
    let mut desc_m = vec![0u8; 0x1000];
    let mut words = [0u64; 64];
    let mut desc = Arena::new("desc", DESC, &mut desc_m).expect("arena");
    let mut cs = Stream::new(&mut words, 92);
    assert_eq!(build_frame(&mut desc, &mut cs, &ctx, &input).unwrap_err(), FrameError::Invalid(INVALID_VARYINGS));
    assert_eq!(cs.words().len(), 0);
    /* an offset the signed Attribute field cannot hold is a bad value, not a
     * gap: its own line, and a different kind */
    let far = [VertexAttribute { offset: 1 << 31, ..attributes[0] }];
    let input = f(DrawCall { zs: DepthStencilState::OFF, colour_mask: 0xf, front_cw: false, attributes: &far, ..draw });
    let mut desc_m = vec![0u8; 0x1000];
    let mut words = [0u64; 64];
    let mut desc = Arena::new("desc", DESC, &mut desc_m).expect("arena");
    let mut cs = Stream::new(&mut words, 92);
    let e = build_frame(&mut desc, &mut cs, &ctx, &input).expect_err("offset");
    assert_eq!(e, FrameError::Invalid(INVALID_ATTRIBUTE_OFFSET));
    assert_eq!(cs.words().len(), 0);
    /* a push block whose address uses bits 56-63 cannot carry the slot count */
    let input = f(draw);
    let mut desc_m = vec![0u8; 0x1000];
    let mut words = [0u64; 64];
    let mut desc = Arena::new("desc", DESC | 1 << 56, &mut desc_m).expect("arena");
    let mut cs = Stream::new(&mut words, 92);
    let e = build_frame(&mut desc, &mut cs, &ctx, &input).expect_err("high push address");
    assert_eq!(e, FrameError::Invalid(INVALID_FAU_ADDRESS));
    /* a register's low-bit tag needs a 64-byte-aligned address; the arena
     * aligns every block it hands out, so only the helper sees one */
    assert_eq!(tagged(0x1000, 7, INVALID_SRT_ALIGN), Ok(0x1007));
    assert_eq!(tagged(0x1008, 7, INVALID_SRT_ALIGN), Err(FrameError::Invalid(INVALID_SRT_ALIGN)));
    assert_eq!(tagged(0x1020, 1, INVALID_BLEND_ALIGN), Err(FrameError::Invalid(INVALID_BLEND_ALIGN)));
    assert_eq!(tagged(0x1000, 64, INVALID_BLEND_ALIGN), Err(FrameError::Invalid(INVALID_BLEND_ALIGN)));
    assert_ne!(INVALID_FAU_ADDRESS, INVALID_SRT_ALIGN);
    assert_ne!(INVALID_SRT_ALIGN, INVALID_BLEND_ALIGN);
    /* a draw whose scissor is empty draws nothing: the frame is the clear */
    let mut desc_m = vec![0u8; 0x1000];
    let mut words = [0u64; 64];
    let mut desc = Arena::new("desc", DESC, &mut desc_m).expect("arena");
    let mut cs = Stream::new(&mut words, 92);
    let input = f(DrawCall {
        zs: DepthStencilState::OFF,
        colour_mask: 0xf,
        front_cw: false,
        cull: Cull::None,
        textures: &[],
        vs_texture: None,
        scissor: Some([10, 10, 0, 5]),
        ..draw
    });
    let f = build_frame(&mut desc, &mut cs, &ctx, &input).expect("builds");
    assert!(f.layout.tiler.is_none() && cs.words().len() == 13);
}

/// The clear colour is Mesa's `pan_pack_color` for RGBA8: rounded half to
/// even, saturated. The trace has blue (0xffff0000); the other values pin
/// the rounding on both sides of a half.
#[test]
fn an_fp16_uniform_word_rounds_to_nearest_even() {
    let h = |x: f32| f32_to_f16(x.to_bits());
    /* exact values, both signs and zero */
    assert_eq!((h(1.0), h(-2.0), h(0.0), h(-0.0), h(0.25)), (0x3c00, 0xc000, 0, 0x8000, 0x3400));
    /* 0.1: the nearest half is 0x2e66 (0.0999755859375) */
    assert_eq!(h(0.1), 0x2e66);
    /* a tie between two halves goes to the even one: 1 + 2^-11 is halfway
     * between 0x3c00 and 0x3c01, 1 + 3 * 2^-11 between 0x3c01 and 0x3c02 */
    assert_eq!((h(1.0 + 2f32.powi(-11)), h(1.0 + 3.0 * 2f32.powi(-11))), (0x3c00, 0x3c02));
    /* the largest half, the first value that rounds past it, infinity */
    assert_eq!((h(65504.0), h(65520.0), h(f32::INFINITY)), (0x7bff, 0x7c00, 0x7c00));
    /* subnormals: 2^-24 is the smallest; half of it ties to 0, three
     * halves to 2; a subnormal that rounds up into the smallest normal */
    assert_eq!((h(2f32.powi(-24)), h(2f32.powi(-25)), h(3.0 * 2f32.powi(-25))), (1, 0, 2));
    assert_eq!(h(2f32.powi(-14) - 2f32.powi(-26)), 0x0400);
    assert_eq!(h(f32::NAN) & 0x7e00, 0x7e00, "a NaN stays a NaN");
}

#[test]
fn the_clear_colour_packs_as_mesa_rounds() {
    let b = build();
    assert_eq!(b.desc[0x80 + 0x30..0x80 + 0x34], 0xffff_0000u32.to_le_bytes());
    let mut desc_m = vec![0u8; 0x200];
    let mut words = [0u64; 32];
    let ctx = Context {
        heap: 0,
        geometry_buffer: 0,
        geometry_buffer_size: 65536,
        sample_positions: SAMPLES,
        tiler_max_levels: 8,
    };
    let mut clear = |rgba: [f32; 4]| -> u32 {
        let mut desc = Arena::new("desc", DESC, &mut desc_m).expect("arena");
        let mut cs = Stream::new(&mut words, 92);
        let input = FrameInput {
            zs: None,
            target: Target {
                gpu: TARGET,
                width: 4,
                height: 4,
                row_stride: 16,
                format: TargetFormat::Rgba8888,
                samples: 1,
            },
            clear_rgba: Some(rgba),
            draw: None,
        };
        build_frame(&mut desc, &mut cs, &ctx, &input).expect("builds");
        u32::from_le_bytes(desc.used()[0xb0..0xb4].try_into().expect("4 bytes"))
    };
    /* 0.5 * 255 = 127.5, a half, to the even 128; 0.25 * 255 = 63.75 to
     * 64; -1 saturates to 0 and 2 to 255; NaN to 0. (127.5 is the only
     * half an f32 input gives exactly; the other side of a half is not
     * reachable from a float that is a multiple of 1/255.) */
    assert_eq!(clear([0.5, 0.25, -1.0, 2.0]), 0xff00_4080);
    assert_eq!(clear([2.0, f32::NAN, 0.0, 1.0]), 0xff00_00ff);
}

/// Every block of a built frame, by name: its GPU address and bytes.
fn all_blocks(b: &Built) -> Vec<(&'static str, u64, Vec<u8>)> {
    let l = b.frame.layout;
    let mut v = Vec::new();
    for (name, blk) in [
        ("Framebuffer + ZS room + Render Target", l.fbd),
        ("Local Storage", l.tls),
        ("Depth/Stencil", l.depth_stencil),
        ("Blend", l.blend),
        ("Attribute", l.attributes),
        ("Vertex Buffer", l.buffers),
        ("VS Sampler", l.vs_sampler),
        ("VS sysval block", l.vs_sysvals),
        ("VS UBO table", l.vs_ubos),
        ("VS push constants", l.vs_push),
        ("FS Sampler", l.fs_sampler),
        ("FS sysval block", l.fs_sysvals),
        ("FS UBO table", l.fs_ubos),
        ("FS push constants", l.fs_push),
        ("Tiler Context", l.tiler),
        ("VS resource table", l.vs_resources),
        ("FS resource table", l.fs_resources),
    ] {
        match blk {
            Some(blk) => {
                let o = (blk.gpu - DESC) as usize;
                v.push((name, blk.gpu, b.desc[o..o + blk.len].to_vec()));
            }
            None => v.push((name, 0, Vec::new())),
        }
    }
    v.push(("Tiler Heap", HEAP_DESC, b.heap[..32].to_vec()));
    v.push(("Sample positions", SAMPLES, b.samples[..256].to_vec()));
    v.push(("VS Shader Program", SPDS, b.spds[..32].to_vec()));
    v.push(("FS Shader Program", SPDS + 32, b.spds[32..64].to_vec()));
    v
}

/// The L1 frame (the board's programs, Mesa 25.0.7: what the board ran in
/// L1) against the same frame with our Valhall back end's programs for
/// tri.vert and tri.frag (Mesa 26.2.1's bytes), every input else equal.
/// Every block and every word is compared; where they differ, each
/// difference is named with its cause and checked to be exactly that:
///
/// * the VS push block holds three more words, the store offsets 4, 8, 12
///   26.2.1 promotes (`va_lower_constants`): 8 words, 4 slots, not 5 / 3;
/// * the FS pushes the four blend constants and 1.0h (`bi_pick_blend_constants`,
///   a promoted constant): a 16-byte sysval block (the blend colour, 0), a
///   sysval UBO in its table and a push block of 5 words in 3 slots, where
///   25.0.7's FS pushed nothing;
/// * every block after them moves 0x40 up (the Tiler Context, both resource
///   tables), so the words and descriptors naming them move too (the FBD's
///   Tiler pointer, d0, d4, d40, d90);
/// * the VS is 152 bytes, padded to 256, so the FS binary starts at +0x100,
///   not +0x80 (the FS SPD's Binary).
#[test]
fn the_frame_with_our_programs_differs_from_l1_only_where_named() {
    use crate::mali::program::StageMeta;
    use crate::shaderc::online::mali::MaliShader;
    use crate::shaderc::online::{codegen, store};
    use crate::shaderc::{SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
    let l1 = build();
    let _k = store::test_keep();
    let one = |stage, src: &str| {
        store::with_scratch(|s| {
            codegen::compile_linked_blocks(stage, src.as_bytes(), 0, &[], s, store::keep()).map(|(sh, _)| sh)
        })
        .expect("the scratch is free")
        .expect("our back end compiles it")
    };
    let vs = one(SHC_STAGE_VERTEX, include_str!("../../tools/mali/testdata/tri.vert"));
    let fs = one(SHC_STAGE_FRAGMENT, include_str!("../../tools/mali/testdata/tri.frag"));
    let vm = StageMeta::of(MaliShader::of(&vs).unwrap()).unwrap();
    let fm = StageMeta::of(MaliShader::of(&fs).unwrap()).unwrap();
    // SAFETY: keep holds both records while `_k` lives.
    let (vc, fc) = unsafe { (super::program::code_bytes(&vs).unwrap(), super::program::code_bytes(&fs).unwrap()) };
    assert_eq!((vc.len(), fc.len()), (256, 128), "each program padded to whole 128 bytes");
    let ours = build_with(vc, vm.meta(), fc, fm.meta());
    let (a, b) = (all_blocks(&l1), all_blocks(&ours));
    let get = |v: &[(&'static str, u64, Vec<u8>)], n: &str| v.iter().find(|x| x.0 == n).cloned().expect(n).clone();
    let differ: Vec<&str> = a.iter().zip(&b).filter(|(x, y)| x.1 != y.1 || x.2 != y.2).map(|(x, _)| x.0).collect();
    assert_eq!(
        differ,
        [
            "Framebuffer + ZS room + Render Target",
            "VS push constants",
            "FS sysval block",
            "FS UBO table",
            "FS push constants",
            "Tiler Context",
            "VS resource table",
            "FS resource table",
            "FS Shader Program",
        ],
        "the blocks the programs change, and no other"
    );
    let moved = 0x40u64;
    let (tl1, tours) = (get(&a, "Tiler Context"), get(&b, "Tiler Context"));
    assert_eq!((tours.1, &tours.2), (tl1.1 + moved, &tl1.2), "the Tiler Context: moved, the same bytes");
    let (rl1, rours) = (get(&a, "VS resource table"), get(&b, "VS resource table"));
    assert_eq!((rours.1, &rours.2), (rl1.1 + moved, &rl1.2), "the VS resource table: moved, the same bytes");
    /* the FBD: only its Tiler pointer (+0x38) */
    let (fl1, fours) =
        (get(&a, "Framebuffer + ZS room + Render Target"), get(&b, "Framebuffer + ZS room + Render Target"));
    let mut want = fl1.2.clone();
    want[0x38..0x40].copy_from_slice(&tours.1.to_le_bytes());
    assert_eq!(fours.2, want);
    /* the VS push block: 25.0.7's five words, then 4, 8, 12 */
    let (pl1, pours) = (get(&a, "VS push constants"), get(&b, "VS push constants"));
    assert_eq!(pours.1, pl1.1);
    assert_eq!(pours.2[..20], pl1.2[..20]);
    assert_eq!(pours.2[20..], [4u32, 8, 12].iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>()[..]);
    /* the FS: the blend colour (0) as its sysval block, named by UBO 0 of
     * its table; the push block of the blend constants and 1.0h */
    let fsys = get(&b, "FS sysval block");
    assert_eq!(fsys.2, [0u8; 16]);
    let ubo = get(&b, "FS UBO table");
    assert_eq!(ubo.1, get(&a, "FS UBO table").1 + 0x20);
    let mut want = vec![0u8; 32];
    want[..8].copy_from_slice(&(0x19u64 | 16 << 32).to_le_bytes());
    want[8..16].copy_from_slice(&fsys.1.to_le_bytes());
    assert_eq!(ubo.2, want, "a Buffer of 16 bytes at the sysval block");
    let fpush = get(&b, "FS push constants");
    assert_eq!(fpush.2, [0u32, 0, 0, 0, 0x3c00, 0].iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>());
    let (sl1, sours) = (get(&a, "FS resource table"), get(&b, "FS resource table"));
    assert_eq!(sours.1, sl1.1 + moved);
    assert_eq!(sours.2[16..], sl1.2[16..], "every entry but the UBO table's");
    assert_eq!(sours.2[..8], (ubo.1 | 1 << 56).to_le_bytes());
    /* the FS SPD: only its Binary (+8) */
    let (dl1, dours) = (get(&a, "FS Shader Program"), get(&b, "FS Shader Program"));
    let mut want = dl1.2.clone();
    want[8..16].copy_from_slice(&(CODE + 0x100).to_le_bytes());
    assert_eq!(dours.2, want);
    assert_eq!(&ours.code[..256], vc);
    assert_eq!(&ours.code[256..384], fc);

    /* the stream: L1's words with the moved addresses and the FAU registers
     * the programs change; FAU_FRAGMENT's value no longer fits 48 bits, so
     * its MOVE48 is two MOVE32s (`cs_move64_to`), one word more */
    let m48 = |d: u32, v: u64| {
        Instr::Move48(CsMove48 { destination: d, immediate: v, ..Default::default() }).encode().unwrap()
    };
    let m32 = |d: u32, v: u32| {
        Instr::Move32(CsMove32 { destination: d, immediate: v, ..Default::default() }).encode().unwrap()
    };
    let mut want = l1.words.clone();
    let (vres, fres) = (rours.1, sours.1);
    assert_eq!(move48(want[3]), (0, rl1.1 | 7));
    want[3] = m48(0, vres | 7);
    assert_eq!(want[5], m32(9, (pl1.1 >> 32) as u32 | 3 << 24));
    want[5] = m32(9, (pours.1 >> 32) as u32 | 4 << 24);
    assert_eq!(move48(want[7]), (4, sl1.1 | 7));
    want[7] = m48(4, fres | 7);
    assert_eq!(move48(want[8]), (12, 0), "25.0.7's FS pushes nothing");
    let fau = fpush.1 | 3 << 56;
    want.splice(8..9, [m32(12, fau as u32), m32(13, (fau >> 32) as u32)]);
    for k in [16, 44] {
        assert_eq!(move48(want[k]).1, tl1.1, "word {k} names the Tiler Context");
        want[k] = m48(move48(want[k]).0, tours.1);
    }
    let differ: Vec<usize> =
        (0..want.len().max(ours.words.len())).filter(|&i| want.get(i) != ours.words.get(i)).collect();
    assert!(differ.is_empty(), "words {differ:?} differ beyond the named causes");
    assert_eq!(ours.words.len(), 52);
}

/// The open frame's program table: the same binary with the same
/// descriptors is uploaded once (the second upload returns the first's
/// addresses and moves neither arena); a different binary, or the same
/// binary whose descriptor differs (here the helper-threads bit), is
/// uploaded again; and a program pushed out of the full table is uploaded
/// again, never taken from a stale entry.
#[test]
fn a_frame_uploads_each_program_once() {
    let (mut code_m, mut spd_m) = (vec![0u8; 0x8000], vec![0u8; 0x2000]);
    let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
    let mut spds = Arena::new("spds", 0x7fff_ffff_d000, &mut spd_m).unwrap();
    let mut cache = ProgramCache::default();
    let bin = |k: u8| {
        let mut b = [0u8; 128];
        b[0] = k;
        b
    };
    let fs = meta(ShaderStage::FRAGMENT, 0);
    let a = upload_program_cached(&mut cache, &mut code, &mut spds, &bin(1), fs).unwrap();
    let tops = (code.top(), spds.top());
    let again = upload_program_cached(&mut cache, &mut code, &mut spds, &bin(1), fs).unwrap();
    assert_eq!((again.binary, again.spd), (a.binary, a.spd));
    assert_eq!((code.top(), spds.top()), tops, "nothing uploaded again");
    assert_eq!(cache.hits, 1);
    let other = upload_program_cached(&mut cache, &mut code, &mut spds, &bin(2), fs).unwrap();
    assert_ne!(other.binary, a.binary, "another binary");
    let helpers = ShaderMeta { requires_helper_threads: true, ..fs };
    let h = upload_program_cached(&mut cache, &mut code, &mut spds, &bin(1), helpers).unwrap();
    assert_ne!(h.spd, a.spd, "the same binary with another descriptor");
    /* push program 1 out of the table */
    for k in 3..3 + PROGRAM_CACHE_ENTRIES as u8 {
        upload_program_cached(&mut cache, &mut code, &mut spds, &bin(k), fs).unwrap();
    }
    let top = code.top();
    let back = upload_program_cached(&mut cache, &mut code, &mut spds, &bin(1), fs).unwrap();
    assert!(code.top() > top && back.binary != a.binary, "uploaded again once out of the table");
    assert_eq!(cache.hits, 1);
}
