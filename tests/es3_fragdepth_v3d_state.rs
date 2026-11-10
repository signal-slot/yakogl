// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D shader record and the binner's CFG_BITS of a `gl_FragDepth`
//! program (plan #340, item `v3d-fragdepth-state`), pinned as words.
//!
//! The online compiler says a program writes depth in its program data
//! (`ShcV3dFs::writes_z`, `disable_ez`; tests/es3_fragdepth_v3d.rs). The
//! back end hands them over as `V3dFsProg` (`src/v3d/backend.rs`
//! `programs`, the two fields copied one for one), and the job builder
//! packs them (`driver/src/v3d/v3d_job.c` `v3d_job_draw`, the 4.2
//! `GL_SHADER_STATE_RECORD`): "fragment shader does Z writes" is bit 8 and
//! "turn off early Z test" bit 9 of the record (byte 1, bits 0 and 1;
//! v3d_packet.xml, `v3dx_draw.c:553-682` `emit_shader_state_record`:
//! `fragment_shader_does_z_writes = prog_data->fs->writes_z`,
//! `turn_off_early_z_test = prog_data->fs->disable_ez`). CFG_BITS
//! (`emit_cfg_bits`, `v3dx_emit.c:294-405`) is 4 bytes after the opcode
//! (96); its "early Z enable" and "early Z updates enable" are bits 24 and
//! 25, byte 3 bits 0 and 1.
//!
//! What runs: the fragment shader of dEQP's `fragdepth.write.const` (and a
//! shader without a depth write) is compiled and linked through
//! `SHC_ONLINE`, as glCompileShader and glLinkProgram reach it; the program
//! data's `writes_z`, `disable_ez`, thread count, varying count and single
//! segment flag go to the REAL job builder (`v3d_job.c`, `v3d_frame.c` and
//! the tile code they link, built here with `cc` into a small program of
//! this test's own, which records one triangle draw and prints the draw's
//! BCL bytes, the shader record and the RCL's first packet). What the
//! builder writes is compared with the literals below. The Rust back end is
//! not in the loop: its `programs` is the field copy named above, which
//! `src/v3d/backend_tests.rs` has no test for, and the job builder is a
//! separate C unit the backend tests stand in for.
//!
//! Pinned today, and a caveat the words show: the job builder never turns
//! early Z on (`v3d_job.c` `emit_cfg_bits` leaves CFG_BITS' two early Z bits
//! 0, and `v3d_job_end` writes `early_z_disable` in the RCL's COMMON
//! packet, "legal, only slower", `v3d_job.h`). So the control, a program
//! that does not write depth, has them clear too: the depth-writing program
//! differs from it in the shader record alone, and the early Z off the
//! program asks for is already what every draw gets.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_fragdepth_v3d_state
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2"), target_os = "linux"))]

use core::ffi::c_char;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

const VS: &str = "#version 300 es\n\
in highp vec4 a_position;\nin highp vec2 a_coord;\nout highp vec2 v_coord;\n\
void main (void)\n{\n    gl_Position = a_position;\n    v_coord = a_coord;\n}\n";

fn fragment(body: &str) -> String {
    format!(
        "#version 300 es\nuniform highp vec4 u_color;\nin highp vec2 v_coord;\n\
         layout(location = 0) out mediump vec4 o_color;\n{body}\n"
    )
}

/// dEQP `fragdepth.write.const`.
const WRITES_DEPTH: &str = "void main (void)\n{\n    o_color = u_color;\n    gl_FragDepth = 0.5;\n}";
/// The same program without the depth write.
const NO_DEPTH: &str = "void main (void)\n{\n    o_color = u_color;\n}";

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> ShcShader {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    assert_eq!(rc, 0, "compile: {}\n{src}", text_of(&log));
    out
}

/// The linked fragment record's program data for `fs`.
fn fs_prog_data(fs: &str) -> yakogl::shaderc::ShcV3dFs {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, VS);
    let mut fso = compile(SHC_STAGE_FRAGMENT, &fragment(fs));
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    assert_eq!(rc, 0, "link: {}", text_of(&log));
    assert_eq!(fso.v3d.linked, 1, "the V3D record is the linked program's");
    fso.v3d.fs
}

// ---- the job builder, built and run ----------------------------------------------

/// One triangle draw through `v3d_job_begin`, `v3d_job_draw`, `v3d_job_end`
/// with a fragment program of the given prog_data fields (no uniforms, no
/// attributes), depth test and write as asked; the draw's BCL bytes, the
/// shader record (40 bytes) and the RCL's first 9 bytes (the COMMON packet)
/// as hex lines.
const HARNESS: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "v3d_frame.h"
#include "v3d_job.h"
#define BO_GPU 0x00100000u
#define BO_SIZE 0x00020000u
static uint8_t bo[BO_SIZE];
int main(int argc, char **argv)
{
	if (argc != 7)
		return 9;
	struct v3d_cl bcl, rcl, ind;
	v3d_cl_init(&bcl, bo, BO_GPU, 0x4000);
	v3d_cl_init(&rcl, bo + 0x4000, BO_GPU + 0x4000, 0x4000);
	v3d_cl_init(&ind, bo + 0x8000, BO_GPU + 0x8000, BO_SIZE - 0x8000);
	struct v3d_fs_prog fs;
	memset(&fs, 0, sizeof(fs));
	fs.code = 0x200000;
	fs.writes_z = atoi(argv[1]);
	fs.disable_ez = atoi(argv[2]);
	fs.threads = atoi(argv[3]);
	fs.num_inputs = atoi(argv[4]);
	fs.single_seg = atoi(argv[5]);
	struct v3d_vs_prog vs;
	memset(&vs, 0, sizeof(vs));
	vs.code = 0x201000;
	vs.threads = 4;
	vs.single_seg = true;
	vs.vpm_output_size = 2;
	vs.vcm_cache_size = 4;
	struct v3d_vs_prog cs = vs;
	cs.code = 0x202000;
	struct v3d_job j;
	struct v3d_job_target t;
	memset(&t, 0, sizeof(t));
	t.width = 200;
	t.height = 100;
	t.address = 0x400000;
	t.stride = 800;
	t.tile_alloc = 0x800000;
	if (!v3d_job_begin(&j, &t, &bcl, &rcl, &ind))
		return 1;
	struct v3d_job_state st;
	memset(&st, 0, sizeof(st));
	st.front_ccw = true;
	st.line_width = 1.0f;
	st.depth_func = V3D_COMPARE_FUNC_ALWAYS;
	st.colormask = 0xf;
	v3d_viewport_gl(0, 0, 200, 100, 0, 1, true, 100, &st.vp);
	if (atoi(argv[6])) {
		st.depth_enabled = true;
		st.depth_writemask = true;
		st.depth_func = V3D_COMPARE_FUNC_LESS;
	}
	struct v3d_job_draw d;
	memset(&d, 0, sizeof(d));
	d.st = &st;
	d.fs = &fs;
	d.vs = &vs;
	d.cs = &cs;
	d.prim = V3D_PRIM_TRIANGLES;
	d.count = 3;
	struct v3d_job_draw_layout lo;
	memset(&lo, 0, sizeof(lo));
	uint32_t b0 = bcl.used;
	if (v3d_job_draw(&j, &d, &lo) != V3D_JOB_OK)
		return 2;
	printf("bcl:");
	for (uint32_t i = b0; i < bcl.used; i++)
		printf(" %02x", bcl.buf[i]);
	printf("\nrec:");
	for (uint32_t i = 0; i < 40; i++)
		printf(" %02x", ind.buf[lo.record - ind.gpu + i]);
	struct v3d_job_clear c;
	memset(&c, 0, sizeof(c));
	c.rgba8 = 0xff000000u;
	c.z = 1.0f;
	if (!v3d_job_end(&j, &c, NULL, NULL))
		return 3;
	printf("\nrcl:");
	for (uint32_t i = 0; i < 9; i++)
		printf(" %02x", rcl.buf[i]);
	printf("\n");
	return 0;
}
"#;

fn harness() -> &'static PathBuf {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_fragdepth_v3d_state");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let src = dir.join(format!("harness-{}.c", std::process::id()));
        let exe = dir.join(format!("harness-{}", std::process::id()));
        std::fs::write(&src, HARNESS).expect("the harness source");
        /* the compiler's own temporary files next to the program: a host
         * whose /tmp is full must not fail the check */
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .arg(&src)
            .args(["driver/src/v3d/v3d_job.c", "driver/src/v3d/v3d_frame.c"])
            .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
            .arg(&exe)
            .status()
            .expect("cc runs");
        assert!(st.success(), "the job builder builds");
        let _ = std::fs::remove_file(&src);
        exe
    })
}

/// What the job builder wrote for one draw.
#[derive(Debug)]
struct Words {
    /// The draw's BCL bytes.
    bcl: Vec<u8>,
    /// The shader record's first 40 bytes.
    rec: Vec<u8>,
    /// The RCL's first packet, TILE_RENDERING_MODE_CFG_COMMON.
    rcl: Vec<u8>,
}

fn hex_line(out: &str, key: &str) -> Vec<u8> {
    let line = out.lines().find_map(|l| l.strip_prefix(key)).unwrap_or_else(|| panic!("a {key} line in\n{out}"));
    line.split_whitespace().map(|b| u8::from_str_radix(b, 16).expect("a hex byte")).collect()
}

fn draw(fs: &yakogl::shaderc::ShcV3dFs, depth_test: bool) -> Words {
    let out = Command::new(harness())
        .args([
            fs.writes_z.to_string(),
            fs.disable_ez.to_string(),
            fs.threads.to_string(),
            fs.num_inputs.to_string(),
            fs.single_seg.to_string(),
            u8::from(depth_test).to_string(),
        ])
        .output()
        .expect("the harness runs");
    assert!(out.status.success(), "the job builder records the draw: {:?}", out.status);
    let text = String::from_utf8(out.stdout).expect("utf-8");
    Words { bcl: hex_line(&text, "bcl:"), rec: hex_line(&text, "rec:"), rcl: hex_line(&text, "rcl:") }
}

/// The CFG_BITS packet of the draw's BCL, the fixed place the builder puts
/// it (after the clip window: `v3d_job_draw` writes CLIP_WINDOW 0x6b, 8
/// bytes, and CFG_BITS follows): opcode 96, four bytes.
fn cfg_bits(w: &Words) -> [u8; 4] {
    assert_eq!(w.bcl[0], 0x6b, "the draw starts with CLIP_WINDOW");
    w.bcl[9..13].try_into().unwrap()
}

// ---- the words ------------------------------------------------------------------

/// The 4.2 shader record (36 bytes, then the next record's start; 40 read),
/// as the builder writes it for this harness's draw: code address 0x200000,
/// 4 threads, no varyings, a triangle with the vertex stages' sizes of the
/// harness. Byte 0: bit 1 enable clipping. Byte 1: bit 0 fragment shader does
/// Z writes (record bit 8), bit 1 turn off early Z test (bit 9). Byte 2: bit
/// 2 disable implicit point/line varyings.
const RECORD_NO_DEPTH: [u8; 40] = [
    0x02, 0x00, 0x04, 0x00, 0x02, 0x01, 0x02, 0x01, 0x00, 0x80, 0x10, 0x00, 0x05, 0x00, 0x20, 0x00, 0x00, 0x81, 0x10,
    0x00, 0x07, 0x10, 0x20, 0x00, 0x00, 0x81, 0x10, 0x00, 0x07, 0x20, 0x20, 0x00, 0x00, 0x81, 0x10, 0x00, 0x00, 0x80,
    0x10, 0x00,
];
const RECORD_WRITES_DEPTH: [u8; 40] = [
    0x02, 0x03, 0x04, 0x00, 0x02, 0x01, 0x02, 0x01, 0x00, 0x80, 0x10, 0x00, 0x05, 0x00, 0x20, 0x00, 0x00, 0x81, 0x10,
    0x00, 0x07, 0x10, 0x20, 0x00, 0x00, 0x81, 0x10, 0x00, 0x07, 0x20, 0x20, 0x00, 0x00, 0x81, 0x10, 0x00, 0x00, 0x80,
    0x10, 0x00,
];

/// CFG_BITS, opcode 96 then 3 bytes. Byte 1 0x07: forward and reverse facing
/// on, clockwise, diamond exit. Byte 2: Z updates (bit 7) and the depth
/// function (bits 4..6; ALWAYS 7, LESS 1). Byte 3: bit 0 early Z enable, bit
/// 1 early Z updates enable, bit 2 stencil, bit 3 blend: all 0.
const CFG_NO_DEPTH_TEST: [u8; 4] = [0x60, 0x07, 0x70, 0x00];
const CFG_DEPTH_TEST_LESS: [u8; 4] = [0x60, 0x07, 0x90, 0x00];

fn zbits(rec: &[u8]) -> (bool, bool) {
    (rec[1] & 1 != 0, rec[1] & 2 != 0)
}

/// The program data the two shaders compile to: the depth writer sets
/// writes_z and disable_ez, the other keeps early tests.
#[test]
fn the_program_data_says_writes_z_and_disable_ez_only_for_the_depth_writer() {
    let w = fs_prog_data(WRITES_DEPTH);
    assert_eq!((w.writes_z, w.disable_ez), (1, 1), "gl_FragDepth = 0.5");
    let n = fs_prog_data(NO_DEPTH);
    assert_eq!((n.writes_z, n.disable_ez), (0, 0), "no depth write");
}

/// The shader record of the depth writer: both bits set, and nothing else of
/// the first four bytes different from the control's.
#[test]
fn the_depth_writers_shader_record_says_writes_z_and_early_z_off() {
    let fs = fs_prog_data(WRITES_DEPTH);
    for depth_test in [false, true] {
        let w = draw(&fs, depth_test);
        assert_eq!(
            zbits(&w.rec),
            (true, true),
            "depth_test {depth_test}: fragment shader does Z writes, turn off early Z"
        );
        assert_eq!(w.rec, RECORD_WRITES_DEPTH, "depth_test {depth_test}: the record");
    }
}

/// The control: a program without a depth write has both bits clear.
#[test]
fn the_control_shader_record_has_both_bits_clear() {
    let fs = fs_prog_data(NO_DEPTH);
    for depth_test in [false, true] {
        let w = draw(&fs, depth_test);
        assert_eq!(zbits(&w.rec), (false, false), "depth_test {depth_test}");
        assert_eq!(w.rec, RECORD_NO_DEPTH, "depth_test {depth_test}: the record");
    }
}

/// The whole record differs between the two programs in exactly the two
/// bits (the rest of the program data and the draw are the same).
#[test]
fn the_two_records_differ_in_the_two_bits_alone() {
    let (a, b) = (fs_prog_data(NO_DEPTH), fs_prog_data(WRITES_DEPTH));
    let (ra, rb) = (draw(&a, true).rec, draw(&b, true).rec);
    /* the thread count, varying count and segment flag come from each
     * program's own data; they are the same here */
    assert_eq!((a.threads, a.num_inputs, a.single_seg), (b.threads, b.num_inputs, b.single_seg));
    let diff: Vec<usize> = (0..ra.len()).filter(|&i| ra[i] != rb[i]).collect();
    assert_eq!(diff, [1], "records {ra:02x?} and {rb:02x?}");
    assert_eq!(ra[1] ^ rb[1], 3, "the two bits");
}

/// CFG_BITS: early Z and early Z updates stay off for both programs, with or
/// without a depth test. Today's literal for the draws pinned.
#[test]
fn cfg_bits_keep_early_z_off_for_both_programs() {
    let (w, n) = (fs_prog_data(WRITES_DEPTH), fs_prog_data(NO_DEPTH));
    for (what, fs) in [("depth writer", &w), ("control", &n)] {
        for depth_test in [false, true] {
            let words = draw(fs, depth_test);
            let c = cfg_bits(&words);
            let want = if depth_test { CFG_DEPTH_TEST_LESS } else { CFG_NO_DEPTH_TEST };
            assert_eq!(c, want, "{what}, depth_test {depth_test}: CFG_BITS");
            assert_eq!(c[3] & 3, 0, "{what}, depth_test {depth_test}: early Z enable and updates enable");
        }
    }
}

/// The RCL's COMMON packet (opcode 0x79, 9 bytes) turns early Z off for every
/// job (`early_z_disable` is bit 6 of byte 6; byte 6 0xc0 is it and the
/// 24-bit depth type's low bit) and is the same for both programs.
#[test]
fn the_rcl_common_packet_disables_early_z_for_both_programs() {
    let (w, n) = (fs_prog_data(WRITES_DEPTH), fs_prog_data(NO_DEPTH));
    let (rw, rn) = (draw(&w, true).rcl, draw(&n, true).rcl);
    assert_eq!(rw, [0x79, 0x00, 0xc8, 0x00, 0x64, 0x00, 0xc0, 0x00, 0x00], "200x100, early Z disabled");
    assert_eq!(rw, rn);
    assert_ne!(rw[6] & 0x40, 0, "early_z_disable");
}
