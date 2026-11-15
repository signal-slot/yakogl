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
//! not in the loop of those tests; the last test goes through it (the GL
//! core and `yakogl_backend_v3d.draw`, the job builder stood in for as in
//! tests/es3_four_rt_v3d.rs) and pins `programs`' field copy: the
//! `V3dFsProg` it hands over has `writes_z` and `disable_ez` set for the
//! writer and clear for the control.
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

/// Through the real V3D back end: the GL core compiles and links the two
/// programs, `yakogl_backend_v3d.draw` records a draw and its `programs`
/// builds the `V3dFsProg` the job builder (a stand-in here, the harness of
/// tests/es3_four_rt_v3d.rs) is handed. Only `writes_z` and `disable_ez` of
/// that record are read.
mod gl_api {
    use core::ffi::{c_char, c_int, c_void, CStr};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Mutex, MutexGuard, Once, OnceLock};
    use yakogl::gl::buffer::glBindBuffer;
    use yakogl::gl::consts::*;
    use yakogl::gl::context;
    use yakogl::gl::draw::*;
    use yakogl::gl::framebuffer::*;
    use yakogl::gl::program::*;
    use yakogl::gl::shader::*;
    use yakogl::gl::state::*;
    use yakogl::gl::types::*;
    use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
    use yakogl::v3d::backend::yakogl_backend_v3d;
    use yakogl::v3d::ffi::*;
    use yakogl::v3d::kick::V3dKick;

    /* ---- the job builder: records, draws nothing ---- */

    #[derive(Default)]
    struct Seen {
        jobs: Vec<V3dJobTarget>,
        clears: Vec<V3dJobClear>,
        /// The vertex count of each draw the job builder was handed.
        draws: Vec<u32>,
        tilings: Vec<Tiling>,
        /// `(writes_z, disable_ez)` of the program record of each draw the job
        /// builder was handed: `programs`' copy, as `V3dFsProg`.
        zbits: Vec<(bool, bool)>,
        log: Vec<String>,
    }

    /// One call of the tile rule: what was asked, the tile it answered.
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Tiling {
        rts: u32,
        msaa: bool,
        max_bpp: u32,
        tile: (u32, u32),
    }

    static SEEN: Mutex<Seen> = Mutex::new(Seen {
        jobs: Vec::new(),
        clears: Vec::new(),
        draws: Vec::new(),
        tilings: Vec::new(),
        zbits: Vec::new(),
        log: Vec::new(),
    });

    fn seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
        f(&mut SEEN.lock().unwrap_or_else(|e| e.into_inner()))
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_begin(
        j: *mut V3dJob,
        t: *const V3dJobTarget,
        bcl: *mut V3dCl,
        rcl: *mut V3dCl,
        ind: *mut V3dCl,
    ) -> bool {
        // SAFETY: the backend passes its live job, target and lists.
        let (j, t) = unsafe { (&mut *j, *t) };
        *j = V3dJob { bcl, rcl, ind, t, draws: 0, defaults: 0, broken: false, pack: V3dJobPack::ZERO };
        // SAFETY: as above.
        unsafe {
            (*bcl).used = 16;
            (*ind).used = 256;
        }
        seen(|s| s.jobs.push(t));
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
        let (j, d) = unsafe { (&mut *j, &*d) };
        j.draws += 1;
        // SAFETY: the job's BCL is live; the draw's slices are as long as it says.
        unsafe {
            (*j.bcl).used += 120;
            seen(|s| s.draws.push(d.count));
            let fs = &*d.fs;
            seen(|s| s.zbits.push((fs.writes_z, fs.disable_ez)));
        }
        V3D_JOB_OK
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_end(
        j: *mut V3dJob,
        c: *const V3dJobClear,
        _tile_list_start: *mut u32,
        _tile_list_end: *mut u32,
    ) -> bool {
        // SAFETY: the backend passes its live job and clear; its lists are live.
        unsafe {
            seen(|s| s.clears.push(*c));
            (*(*j).bcl).used += 1;
            (*(*j).rcl).used = 200;
        }
        true
    }

    extern "C" {
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    /// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
    /// object of this test's own, reached through the handle only (RTLD_LOCAL).
    fn lib() -> *mut c_void {
        static L: OnceLock<usize> = OnceLock::new();
        *L.get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_fragdepth_state");
            std::fs::create_dir_all(&dir).expect("the scratch directory");
            let so = dir.join(format!("v3d_fragdepth_state-{}.so", std::process::id()));
            /* the compiler's own temporary files next to the object */
            let st = Command::new("cc")
                .current_dir(&root)
                .env("TMPDIR", &dir)
                .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
                .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
                .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
                .arg("-Ithird_party/mesa-26.2.1/include")
                .args([
                    "driver/src/v3d/v3d_tex.c",
                    "driver/src/v3d/v3d_tiling_mesa.c",
                    "driver/src/v3d/v3d_frame.c",
                    "-lm",
                    "-o",
                ])
                .arg(&so)
                .status()
                .expect("cc runs");
            assert!(st.success(), "driver/src/v3d/v3d_tex.c builds");
            let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
            // SAFETY: a path of a library this test just built; RTLD_NOW.
            let h = unsafe { dlopen(path.as_ptr(), 2) };
            assert!(!h.is_null(), "the layout code loads");
            let _ = std::fs::remove_file(&so);
            h as usize
        }) as *mut c_void
    }

    fn sym(name: &str) -> *mut c_void {
        let c = CString::new(name).unwrap();
        // SAFETY: a handle `lib` made and a NUL-terminated name.
        let p = unsafe { dlsym(lib(), c.as_ptr()) };
        assert!(!p.is_null(), "{name} in v3d_tex.c");
        p
    }

    /// The trampolines: the library's `extern "C"` texture functions
    /// (src/v3d/ffi.rs), each handed to the real C.
    macro_rules! trampoline {
        ($name:ident ( $($arg:ident : $ty:ty),* ) $(-> $ret:ty)?) => {
            #[no_mangle]
            unsafe extern "C" fn $name($($arg: $ty),*) $(-> $ret)? {
                // SAFETY: the C function's own signature, as src/v3d/ffi.rs declares it.
                let f: unsafe extern "C" fn($($ty),*) $(-> $ret)? =
                    unsafe { std::mem::transmute(sym(stringify!($name))) };
                // SAFETY: the library's call, passed on.
                unsafe { f($($arg),*) }
            }
        };
    }

    /* The signatures are src/v3d/ffi.rs's, kept as written (see
     * tests/depth_resolve_v3d.rs: rustfmt releases wrap these differently). */
    #[rustfmt::skip]
    mod trampolines {
        use super::*;

        trampoline!(v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
        trampoline!(v3d_tex_layout_msaa(width: u32, height: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
        trampoline!(v3d_tex_store_level(image: *mut u8, l: *const V3dTexLayout, level: u32, src: *const u8, stride: u32));
        trampoline!(v3d_tex_tfu_pack(l: *const V3dTexLayout, ttype: u32, src: u32, stride: u32, dst: u32,
                                     j: *mut V3dTexTfuJob) -> bool);
        trampoline!(v3d_tex_tfu_pack_mipmap(l: *const V3dTexLayout, ttype: u32, image: u32, j: *mut V3dTexTfuJob) -> bool);
        trampoline!(v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32);
        trampoline!(v3d_tex_layers_size(l: *const V3dTexLayout, layers: u32, out: *mut u32) -> bool);
        trampoline!(v3d_tex_store_box(image: *mut u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                      src: *const u8, stride: u32) -> bool);
        trampoline!(v3d_tex_load_box(image: *const u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                     dst: *mut u8, stride: u32) -> bool);
        trampoline!(v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout));
        trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
        trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
        trampoline!(v3d_frame_alloc_sizes(t: *const V3dTiling, draws: u32, page: u32, ta: *mut u32, ts: *mut u32));
    }

    /// The real tile rule (driver/src/v3d/v3d_frame.c), each call recorded:
    /// the render-target count, MSAA and largest internal bpp the back end
    /// asked for, and the tile it got.
    fn tiling(rts: u32, msaa: bool, max_bpp: u32, t: *const V3dTiling) {
        // SAFETY: the library filled `t` just before.
        let t = unsafe { *t };
        seen(|s| s.tilings.push(Tiling { rts, msaa, max_bpp, tile: (t.tile_w, t.tile_h) }));
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling) {
        // SAFETY: the C function's signature, filling the backend's out-parameter.
        let f: unsafe extern "C" fn(u32, u32, *mut V3dTiling) = unsafe { std::mem::transmute(sym("v3d_frame_tiling")) };
        // SAFETY: as above.
        unsafe { f(width, height, t) };
        tiling(1, false, 0, t);
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling) {
        // SAFETY: the C function's signature, filling the backend's out-parameter.
        let f: unsafe extern "C" fn(u32, u32, u32, *mut V3dTiling) =
            unsafe { std::mem::transmute(sym("v3d_frame_tiling_rts")) };
        // SAFETY: as above.
        unsafe { f(width, height, rts, t) };
        tiling(rts, false, 0, t);
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling) {
        // SAFETY: the C function's signature, filling the backend's out-parameter.
        let f: unsafe extern "C" fn(u32, u32, u32, bool, *mut V3dTiling) =
            unsafe { std::mem::transmute(sym("v3d_frame_tiling_ms")) };
        // SAFETY: as above.
        unsafe { f(width, height, rts, msaa, t) };
        tiling(rts, msaa, 0, t);
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_bpp(
        width: u32,
        height: u32,
        rts: u32,
        msaa: bool,
        max_bpp: u32,
        t: *mut V3dTiling,
    ) {
        // SAFETY: the C function's signature, filling the backend's out-parameter.
        let f: unsafe extern "C" fn(u32, u32, u32, bool, u32, *mut V3dTiling) =
            unsafe { std::mem::transmute(sym("v3d_frame_tiling_bpp")) };
        // SAFETY: as above.
        unsafe { f(width, height, rts, msaa, max_bpp, t) };
        tiling(rts, msaa, max_bpp, t);
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool {
        // SAFETY: the C function's signature.
        let f: unsafe extern "C" fn(*mut u8, *const V3dTexState) -> bool =
            unsafe { std::mem::transmute(sym("v3d_tex_pack_state")) };
        // SAFETY: the library's call, passed on; `s` is its live state.
        unsafe { f(out, s) }
    }

    /* ---- the platform: memory that answers, jobs that finish at once ---- */

    struct Alloc {
        mem: Box<[u64]>,
        size: usize,
    }
    struct Platform {
        next: u64,
        allocs: BTreeMap<u64, Alloc>,
        submitted: usize,
    }
    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(|| Platform { next: 0, allocs: BTreeMap::new(), submitted: 0 }))
    }

    unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        with_platform(|p| {
            if size == 0 || size > 256 << 20 {
                return -12;
            }
            let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
            let gpu = 0x0100_0000 + p.next;
            p.next += (size as u64).div_ceil(4096) * 4096;
            let cpu = mem.as_mut_ptr() as *mut c_void;
            // SAFETY: the caller passes a writable descriptor.
            unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
            p.allocs.insert(gpu, Alloc { mem, size });
            0
        })
    }
    unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
        // SAFETY: the caller passes its descriptor.
        let m = unsafe { *mem };
        if !m.impl_.is_null() {
            with_platform(|p| p.allocs.remove(&m.gpu));
        }
        0
    }
    unsafe extern "C" fn p_gpu_write(mem: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
        // SAFETY: the caller passes its descriptor.
        let m = unsafe { *mem };
        if m.cpu.is_null() || off.checked_add(n).is_none_or(|e| e > m.size) {
            return -22;
        }
        // SAFETY: inside the allocation (checked above).
        unsafe { core::ptr::copy_nonoverlapping(src as *const u8, (m.cpu as *mut u8).add(off), n) };
        0
    }
    unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}
    unsafe extern "C" fn p_submit(_kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<V3dKick>() {
            return -22;
        }
        with_platform(|p| {
            p.submitted += 1;
            p.submitted as i64
        })
    }
    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }
    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.submitted as i64)
    }
    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_VPM_BYTES};
        if what == OSAL_QUERY_V3D_VPM_BYTES && out_sz == 4 {
            // SAFETY: a u32 out-parameter.
            unsafe { *(out as *mut u32) = 65536 };
            return 0;
        }
        if what == OSAL_QUERY_V3D_CPU_OF_GPU && out_sz == size_of::<OsalV3dCpuOf>() {
            // SAFETY: the backend's in/out struct of that size.
            let q = unsafe { &mut *(out as *mut OsalV3dCpuOf) };
            return with_platform(|p| match p.allocs.range(..=q.gpu).next_back() {
                Some((base, a)) if q.gpu + q.bytes <= base + a.size as u64 => {
                    // SAFETY: inside the allocation.
                    q.cpu = unsafe { (a.mem.as_ptr() as *mut u8).add((q.gpu - base) as usize) } as *mut c_void;
                    0
                }
                _ => -22,
            });
        }
        if what == OSAL_QUERY_V3D_BIN_POOL {
            return -61;
        }
        -22
    }
    /// The library logs every line as `log("%s", line)` (`osal::log_str`).
    unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
        // SAFETY: the library passes a NUL-terminated line.
        let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
        seen(|x| x.log.push(s));
    }
    unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
        0
    }

    type LogFn = unsafe extern "C" fn(*const c_char, ...);

    fn osal() -> &'static OsalOps {
        // SAFETY: see p_log2: the library only ever calls it with ("%s", line).
        let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log2) };
        Box::leak(Box::new(OsalOps {
            init: Some(p_init),
            shutdown: None,
            gpu_alloc: Some(p_gpu_alloc),
            gpu_free: Some(p_gpu_free),
            mmu_map: None,
            map_phys: None,
            gpu_write: Some(p_gpu_write),
            gpu_read: None,
            cache_flush: Some(p_cache),
            cache_invd: Some(p_cache),
            submit: Some(p_submit),
            wait: Some(p_wait),
            completed: Some(p_completed),
            reg_read: None,
            reg_write: None,
            query: Some(p_query),
            log: Some(log),
            wait_vblank: None,
            present: None,
            lock: None,
            unlock: None,
            thread_id: None,
            display_busy: None,
        }))
    }

    /* ---- the context ---- */

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();

    /// The one context and back end of the module; the tests take turns and
    /// each starts with nothing recorded and nothing pending.
    fn rig() -> MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ONCE.call_once(|| {
            // SAFETY: statics; the lock serialises every GL call of the module.
            unsafe {
                osal_set_ops(osal());
                yakogl::backend::g_backend = &yakogl_backend_v3d;
                assert_eq!(yakogl::backend::init(), 0, "{:?}", seen(|s| s.log.clone()));
                assert!(!context::gl_context_create(32, 32).is_null(), "{:?}", seen(|s| s.log.clone()));
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        seen(|s| *s = Seen::default());
        yakogl::backend::HOST_DRAW_TARGETS.store(0, core::sync::atomic::Ordering::Relaxed);
        g
    }

    /* ---- the framebuffer, the program, the draw ---- */

    /// GL_NO_ERROR, or the first error of the queue (the queue is drained).
    fn err() -> GLenum {
        // SAFETY: plain GL calls.
        unsafe {
            let e = glGetError();
            while glGetError() != GL_NO_ERROR {}
            e
        }
    }

    /// An 8x8 renderbuffer of `format` on colour attachment `point` of the
    /// bound framebuffer; its name.
    fn attach(point: GLenum, format: GLenum) -> GLuint {
        let mut rb = 0;
        // SAFETY: plain GL calls with room for one name.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, format, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb);
        }
        rb
    }

    /// A bound framebuffer of 8x8 renderbuffers of `formats` on colour
    /// attachments 0.., all of them draw buffers; deleted when dropped.
    struct Fbo {
        fb: GLuint,
        rbs: Vec<GLuint>,
    }

    impl Fbo {
        fn new(formats: &[GLenum]) -> Fbo {
            let mut fb = 0;
            // SAFETY: room for one name.
            unsafe {
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
            }
            let rbs: Vec<GLuint> =
                formats.iter().enumerate().map(|(i, f)| attach(GL_COLOR_ATTACHMENT0 + i as GLenum, *f)).collect();
            let bufs: Vec<GLenum> = (0..formats.len()).map(|i| GL_COLOR_ATTACHMENT0 + i as GLenum).collect();
            // SAFETY: `bufs` has `len` enums.
            unsafe {
                glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
                glViewport(0, 0, 8, 8);
            }
            let log = seen(|s| s.log.clone());
            assert_eq!(err(), GL_NO_ERROR, "{formats:#x?}: {log:?}");
            // SAFETY: plain GL call.
            let st = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
            assert_eq!(st, GL_FRAMEBUFFER_COMPLETE, "{formats:#x?}: {log:?}");
            Fbo { fb, rbs }
        }
    }

    impl Drop for Fbo {
        fn drop(&mut self) {
            // SAFETY: names this test made.
            unsafe {
                glDisable(GL_SCISSOR_TEST);
                glColorMask(1, 1, 1, 1);
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDeleteFramebuffers(1, &self.fb);
                glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
                glUseProgram(0);
            }
        }
    }

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

    /// The current program: the file's vertex shader and the fragment shader `fs`.
    fn program(fs: &str) {
        let v = shader(GL_VERTEX_SHADER, super::VS);
        let f = shader(GL_FRAGMENT_SHADER, fs);
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1);
            glUseProgram(p);
        }
    }

    /// One triangle over the framebuffer, flushed. Returns what the job
    /// builder recorded.
    fn draw() -> Seen {
        static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
        seen(|s| *s = Seen::default());
        // SAFETY: TRI is static and outlives the draw.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
        }
        let log = seen(|s| s.log.clone());
        assert_eq!(err(), GL_NO_ERROR, "{log:?}");
        seen(core::mem::take)
    }

    /// `(writes_z, disable_ez)` the job builder got for one draw of `fs`.
    pub fn zbits_of(fs: &str) -> (bool, bool) {
        let _g = rig();
        let fbo = Fbo::new(&[GL_RGBA8]);
        program(fs);
        let s = draw();
        drop(fbo);
        assert_eq!((s.jobs.len(), s.draws.clone()), (1, vec![3]), "one job, one draw: {:?}", s.log);
        assert_eq!(s.zbits.len(), 1, "{:?}", s.log);
        s.zbits[0]
    }
}

/// `programs` copies the compiler's `writes_z` and `disable_ez` one for one:
/// both set for the `gl_FragDepth` writer, both clear for the control.
#[test]
fn the_back_end_hands_the_job_builder_writes_z_and_disable_ez_only_for_the_depth_writer() {
    assert_eq!(gl_api::zbits_of(&fragment(WRITES_DEPTH)), (true, true), "gl_FragDepth = 0.5");
    assert_eq!(gl_api::zbits_of(&fragment(NO_DEPTH)), (false, false), "no depth write");
}
