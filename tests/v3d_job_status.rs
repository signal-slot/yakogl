// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A job-builder refusal names its cause (plan #399, item jobstatus-apply;
//! #1261, docs/notes/v3d-refusals.md E3).
//!
//! `v3d_job_draw` used to answer one `V3D_JOB_REFUSED` for every check that
//! stopped a draw (`draw_is_supported`, `record_draw`, a field that did not
//! fit, a broken job), so a refusal left on the Pi 4 console had no named
//! cause. It now answers one result per cause (`driver/src/v3d/v3d_job.h`),
//! and the V3D back end prints [`v3d_job_refusal`]'s text for it in its
//! "draw refused:" line, still answering -ENOTSUP (src/v3d/backend.rs
//! `record_arrays_and_draw`; src/v3d/backend_tests.rs pins that side).
//!
//! Here the REAL job builder runs: `driver/src/v3d/v3d_job.c` and the files
//! it needs, built with `cc` into one shared object and loaded with `dlopen`
//! (as tests/v3d_frame_rt4.rs does). For each cause a draw that triggers it
//! is handed to `v3d_job_draw`: the result is the cause's, its text is the
//! cause's name, and the lists are left as they stood. Which draws are
//! refused is unchanged: the same draws as before, only the result differs.
//! `V3D_JOB_UNRECORDED` has no draw that reaches it (every stop of
//! `record_draw` today sets a list's `bad_field` or `overflow`); it is a
//! guard, and only its name is pinned.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_job_status
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "gpu-v3d",
    not(feature = "report-es2"),
    target_os = "linux"
))]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use yakogl::v3d::ffi::*;

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_job_status");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_job_status-{}.so", std::process::id()));
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
        // SAFETY: a shared object this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the job builder loads");
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: the handle lib() opened; a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in v3d_job.c");
    p
}

type BeginFn = unsafe extern "C" fn(*mut V3dJob, *const V3dJobTarget, *mut V3dCl, *mut V3dCl, *mut V3dCl) -> bool;
type DrawFn = unsafe extern "C" fn(*mut V3dJob, *const V3dJobDraw, *mut c_void) -> i32;

const W: u32 = 200;
const H: u32 = 100;
const LIST_BYTES: usize = 16 * 1024;

/// The programs, state, attributes and uniform blocks one draw points into:
/// job_check.c's good draw (part 2), a triangle with two attribute records.
struct Parts {
    fs: V3dFsProg,
    vs: V3dVsProg,
    cs: V3dVsProg,
    st: V3dJobState,
    attrs: [V3dVertexAttr; 2],
    words: [u32; 4],
}

const NO_UNIFORMS: *const V3dUniform = core::ptr::null();

impl Parts {
    fn new() -> Parts {
        let vs = V3dVsProg {
            code: 0x0020_1000,
            threads: 4,
            single_seg: true,
            vpm_input_size: 0,
            vpm_output_size: 2,
            separate_segments: false,
            vcm_cache_size: 4,
            uses_vid: false,
            uses_iid: false,
            uses_biid: false,
            vattr_sizes: [4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            uniforms: NO_UNIFORMS,
            num_uniforms: 0,
        };
        let st = V3dJobState {
            vp: V3dViewport {
                scale: [W as f32 / 2.0, H as f32 / 2.0, 0.5],
                translate: [W as f32 / 2.0, H as f32 / 2.0, 0.5],
            },
            front_ccw: true,
            line_width: 1.0,
            depth_func: V3D_COMPARE_FUNC_ALWAYS,
            colormask: 0xf,
            ..Default::default()
        };
        Parts {
            fs: V3dFsProg {
                code: 0x0020_0000,
                threads: 4,
                single_seg: false,
                num_inputs: 4,
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
                uniforms: NO_UNIFORMS,
                num_uniforms: 0,
            },
            vs,
            cs: V3dVsProg { code: 0x0020_2000, ..vs },
            st,
            attrs: [
                V3dVertexAttr { address: 0x0030_0000, stride: 32, components: 4, divisor: 0 },
                V3dVertexAttr { address: 0x0030_0010, stride: 32, components: 4, divisor: 0 },
            ],
            words: [0; 4],
        }
    }

    fn draw(&self) -> V3dJobDraw {
        V3dJobDraw {
            st: &self.st,
            fs: &self.fs,
            vs: &self.vs,
            cs: &self.cs,
            vs_storage: V3dUniformStorage { words: self.words.as_ptr(), count: 4 },
            fs_storage: V3dUniformStorage { words: self.words.as_ptr(), count: 4 },
            attrs: self.attrs.as_ptr(),
            num_attrs: 2,
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
        }
    }
}

/// One job on its own lists. The buffers are boxed so the lists' pointers
/// stay put.
struct Job {
    bufs: Box<[[u8; LIST_BYTES]; 3]>,
    cls: Box<[V3dCl; 3]>,
    job: Box<V3dJob>,
}

impl Job {
    /// A job begun on a W x H target (`width` 0: an empty target, which
    /// v3d_job_begin refuses and leaves broken). `true` when it began.
    fn begin(width: u32) -> (Job, bool) {
        let mut bufs = Box::new([[0u8; LIST_BYTES]; 3]);
        let mut cls = Box::new([V3dCl::ZERO; 3]);
        for (i, (cl, buf)) in cls.iter_mut().zip(bufs.iter_mut()).enumerate() {
            *cl = V3dCl {
                buf: buf.as_mut_ptr(),
                gpu: 0x0010_0000 + 0x0001_0000 * i as u32,
                size: LIST_BYTES as u32,
                used: 0,
                overflow: false,
                bad_field: false,
            };
        }
        let t = V3dJobTarget {
            width,
            height: H,
            address: 0x0040_0000,
            stride: W * 4,
            tile_alloc: 0x0080_0000,
            ..Default::default()
        };
        let mut job = Box::new(V3dJob::ZERO);
        let [bcl, rcl, ind] = &mut *cls;
        // SAFETY: v3d_job.c's v3d_job_begin; the lists point into `bufs`,
        // which the returned Job keeps alive.
        let ok = unsafe {
            let begin: BeginFn = std::mem::transmute(sym("v3d_job_begin"));
            begin(&mut *job, &t, bcl, rcl, ind)
        };
        (Job { bufs, cls, job }, ok)
    }

    /// The lists' state: what a refused draw must leave as it found it.
    fn lists(&self) -> Vec<(u32, bool, bool, Vec<u8>)> {
        self.cls
            .iter()
            .zip(self.bufs.iter())
            .map(|(cl, buf)| (cl.used, cl.overflow, cl.bad_field, buf[..cl.used as usize].to_vec()))
            .collect()
    }

    fn draw(&mut self, d: &V3dJobDraw) -> i32 {
        // SAFETY: v3d_job.c's v3d_job_draw on this job's live lists; every
        // pointer in `d` is to a Parts the caller keeps for the call.
        unsafe {
            let draw: DrawFn = std::mem::transmute(sym("v3d_job_draw"));
            draw(&mut *self.job, d, core::ptr::null_mut())
        }
    }
}

/// `d` changed by `edit` is refused with `want`: the result, the name the
/// refusal line prints, and the lists left as they stood. A good draw still
/// goes in after it.
fn refused(what: &str, want: i32, name: &str, edit: impl FnOnce(&mut Parts, &mut V3dJobDraw)) {
    let (mut job, began) = Job::begin(W);
    assert!(began, "the job begins");
    let good = Parts::new();
    assert_eq!(job.draw(&good.draw()), V3D_JOB_OK, "{what}: the good draw goes in");
    let mut p = Parts::new();
    let mut d = p.draw();
    /* `d` points into `p` (raw pointers), so an edit of either shows */
    edit(&mut p, &mut d);
    let before = job.lists();
    let r = job.draw(&d);
    assert_eq!(r, want, "{what}: v3d_job_draw's result");
    assert!(r >= V3D_JOB_REFUSED, "{what}: a refusal (V3D_JOB_IS_REFUSAL)");
    assert_eq!(v3d_job_refusal(r), name, "{what}: the refusal line's text");
    assert!(name.starts_with("the job builder refused the draw ("), "{what}: the prefix the abort tools read");
    assert!(job.lists() == before, "{what}: the lists are left as they stood");
    assert_eq!(job.job.draws, 1, "{what}: only the good draw is recorded");
    assert_eq!(job.draw(&good.draw()), V3D_JOB_OK, "{what}: a good draw still goes in after the refusal");
}

#[test]
fn a_good_draw_goes_in() {
    let (mut job, began) = Job::begin(W);
    assert!(began);
    let p = Parts::new();
    assert_eq!(job.draw(&p.draw()), V3D_JOB_OK);
    assert_eq!(job.job.draws, 1);
}

#[test]
fn a_broken_job_names_broken() {
    let (mut job, began) = Job::begin(0);
    assert!(!began, "an empty target: v3d_job_begin refuses it and the job is broken");
    let p = Parts::new();
    let r = job.draw(&p.draw());
    assert_eq!(r, V3D_JOB_BROKEN);
    assert_eq!(v3d_job_refusal(r), "the job builder refused the draw (an earlier step broke the job)");
}

const NO_PROGRAM: &str =
    "the job builder refused the draw (no state or program, or a thread count other than 1, 2 or 4)";

#[test]
fn a_missing_program_names_no_program() {
    refused("no fragment program", V3D_JOB_NO_PROGRAM, NO_PROGRAM, |_, d| d.fs = core::ptr::null());
}

#[test]
fn a_thread_count_other_than_1_2_4_names_no_program() {
    refused("an FS of 3 threads", V3D_JOB_NO_PROGRAM, NO_PROGRAM, |p, _| p.fs.threads = 3);
    refused("a CS of 0 threads (never filled)", V3D_JOB_NO_PROGRAM, NO_PROGRAM, |p, _| p.cs.threads = 0);
}

#[test]
fn more_than_64_fs_inputs_names_fs_inputs() {
    const NAME: &str = "the job builder refused the draw (more than 64 fragment input scalars)";
    refused("65 fragment inputs", V3D_JOB_FS_INPUTS, NAME, |p, _| p.fs.num_inputs = 65);
    /* 64 is the cap, not over it */
    let (mut job, _) = Job::begin(W);
    let mut p = Parts::new();
    p.fs.num_inputs = 64;
    assert_eq!(job.draw(&p.draw()), V3D_JOB_OK, "64 fragment inputs go in");
}

#[test]
fn a_shape_the_builder_does_not_write_names_refused() {
    const NAME: &str = "the job builder refused the draw (a uniform, program or primitive it does not write)";
    refused("an unknown primitive", V3D_JOB_REFUSED, NAME, |_, d| d.prim = V3D_PRIM_TRIANGLE_FAN + 1);
    refused("no vertices", V3D_JOB_REFUSED, NAME, |_, d| d.count = 0);
    refused("an attribute read with no record", V3D_JOB_REFUSED, NAME, |_, d| d.num_attrs = 1);
    refused("an attribute of 5 components", V3D_JOB_REFUSED, NAME, |p, _| p.attrs[1].components = 5);
    refused("an unknown index type", V3D_JOB_REFUSED, NAME, |_, d| {
        d.indexed = true;
        d.index_type = 3;
    });
}

#[test]
fn a_value_outside_its_field_names_bad_field() {
    const NAME: &str =
        "the job builder refused the draw (a value outside its packet field, or a uniform it does not write)";
    static OUTSIDE: [V3dUniform; 1] = [V3dUniform { contents: V3D_QUNIFORM_UNIFORM, data: 4 }];
    static UNKNOWN: [V3dUniform; 1] = [V3dUniform { contents: 77, data: 0 }];
    refused("a uniform word outside the storage", V3D_JOB_BAD_FIELD, NAME, |p, _| {
        p.fs.uniforms = OUTSIDE.as_ptr();
        p.fs.num_uniforms = 1;
    });
    refused("an unknown uniform contents", V3D_JOB_BAD_FIELD, NAME, |p, _| {
        p.vs.uniforms = UNKNOWN.as_ptr();
        p.vs.num_uniforms = 1;
    });
    refused("gl_PointCoord drawn as triangles", V3D_JOB_BAD_FIELD, NAME, |p, _| {
        p.fs.uses_implicit_point_line_varyings = true;
    });
    refused("a viewport offset outside VIEWPORT_OFFSET", V3D_JOB_BAD_FIELD, NAME, |p, _| {
        p.st.vp.translate[0] = 1.0e9;
    });
}

#[test]
fn every_cause_has_its_own_name() {
    let causes =
        [V3D_JOB_REFUSED, V3D_JOB_BROKEN, V3D_JOB_NO_PROGRAM, V3D_JOB_FS_INPUTS, V3D_JOB_BAD_FIELD, V3D_JOB_UNRECORDED];
    let names: Vec<&str> = causes.iter().map(|&c| v3d_job_refusal(c)).collect();
    for (i, n) in names.iter().enumerate() {
        assert!(n.starts_with("the job builder refused the draw ("), "{n}");
        assert!(!names[..i].contains(n), "{n}: named twice");
        assert_ne!(*n, v3d_job_refusal(99), "cause {} has a name of its own", causes[i]);
    }
    assert_eq!(
        v3d_job_refusal(V3D_JOB_UNRECORDED),
        "the job builder refused the draw (it stopped with no field named and no list full)"
    );
    /* the Rust constants are v3d_job.h's */
    let h = include_str!("../driver/src/v3d/v3d_job.h");
    for (name, v) in [
        ("V3D_JOB_OK", V3D_JOB_OK),
        ("V3D_JOB_FULL", V3D_JOB_FULL),
        ("V3D_JOB_REFUSED", V3D_JOB_REFUSED),
        ("V3D_JOB_BROKEN", V3D_JOB_BROKEN),
        ("V3D_JOB_NO_PROGRAM", V3D_JOB_NO_PROGRAM),
        ("V3D_JOB_FS_INPUTS", V3D_JOB_FS_INPUTS),
        ("V3D_JOB_BAD_FIELD", V3D_JOB_BAD_FIELD),
        ("V3D_JOB_UNRECORDED", V3D_JOB_UNRECORDED),
    ] {
        let line = h
            .lines()
            .find(|l| l.split_whitespace().take(2).eq(["#define", name]))
            .unwrap_or_else(|| panic!("{name} in v3d_job.h"));
        let c: i32 = line.split_whitespace().nth(2).and_then(|s| s.parse().ok()).expect("a number");
        assert_eq!(c, v, "{name}");
    }
}
