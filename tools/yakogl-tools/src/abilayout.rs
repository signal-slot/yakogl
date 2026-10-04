// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools abi-layout <powervr-bxs-am62p/src>`: generate the ABI layout
//! pins shared by the C headers and the Rust structs.
//!
//! The #[repr(C)] structs of the crate are described to C code by the headers
//! (include/gles_osal.h, include/gles_shaderc.h, src/backend.h,
//! src/gl_context.h, src/pvr/pvr_backend.h and the driver's pvr_render.h).
//! This measures every struct and field with the host C compiler and writes
//! the numbers twice:
//!
//!     src/abi_layout_check.c   _Static_assert(sizeof/offsetof ...) on the C side
//!     src/abi_layout.rs        const assertions (size_of / offset_of!) on the Rust side
//!
//! so a change to either side that is not mirrored in the other fails the
//! build. All pinned types use fixed-width integers, floats, bool and
//! pointers, so the numbers are those of every LP64 little-endian target.
//!
//! Everything is measured once per GPU ([`GPUS`]), because `struct shc_shader`
//! carries a GC7000 half on a Vivante build and a V3D half on a V3D build, and
//! every struct that embeds it grows with it. A number every build agrees on
//! is pinned once; where the builds differ, or where a field exists in some
//! builds only, each GPU's number is pinned behind that GPU's cargo feature
//! and C define (the PowerVR, the default build, has neither: its gate is
//! "none of the others").

use crate::py::{self, g, re};
use crate::{Error, R};
use std::process::Command;

const USAGE: &str = "usage: yakogl-tools abi-layout <powervr-bxs-am62p/src>      (from the repository root)";

/// Rust file, Rust struct, Rust path, C struct.
pub const STRUCTS: &[(&str, &str, &str, &str)] = &[
    ("src/osal/mod.rs", "OsalGpuMem", "crate::osal::OsalGpuMem", "struct osal_gpu_mem"),
    ("src/osal/mod.rs", "OsalOps", "crate::osal::OsalOps", "struct osal_ops"),
    ("src/osal/render.rs", "PvrRenderStatic", "crate::osal::render::PvrRenderStatic", "struct pvr_render_static"),
    (
        "src/osal/render.rs",
        "PvrRenderGeomRegs",
        "crate::osal::render::PvrRenderGeomRegs",
        "struct pvr_render_geom_regs",
    ),
    (
        "src/osal/render.rs",
        "PvrRenderFragRegs",
        "crate::osal::render::PvrRenderFragRegs",
        "struct pvr_render_frag_regs",
    ),
    ("src/osal/render.rs", "PvrRenderKick", "crate::osal::render::PvrRenderKick", "struct pvr_render_kick"),
    ("src/shaderc/mod.rs", "ShcVarying", "crate::shaderc::ShcVarying", "struct shc_varying"),
    ("src/shaderc/mod.rs", "ShcAttrib", "crate::shaderc::ShcAttrib", "struct shc_attrib"),
    ("src/shaderc/mod.rs", "ShcAttribBinding", "crate::shaderc::ShcAttribBinding", "struct shc_attrib_binding"),
    ("src/shaderc/mod.rs", "ShcAttribMap", "crate::shaderc::ShcAttribMap", "struct shc_attrib_map"),
    ("src/shaderc/mod.rs", "ShcUniform", "crate::shaderc::ShcUniform", "struct shc_uniform"),
    ("src/shaderc/mod.rs", "ShcV3dUniform", "crate::shaderc::ShcV3dUniform", "struct shc_v3d_uniform"),
    ("src/shaderc/mod.rs", "ShcV3dFs", "crate::shaderc::ShcV3dFs", "struct shc_v3d_fs"),
    ("src/shaderc/mod.rs", "ShcV3dVs", "crate::shaderc::ShcV3dVs", "struct shc_v3d_vs"),
    ("src/shaderc/mod.rs", "ShcV3d", "crate::shaderc::ShcV3d", "struct shc_v3d"),
    ("src/shaderc/mod.rs", "ShcShader", "crate::shaderc::ShcShader", "struct shc_shader"),
    ("src/shaderc/mod.rs", "ShcBackend", "crate::shaderc::ShcBackend", "struct shc_backend"),
    ("src/backend.rs", "GlesAttrib", "crate::backend::GlesAttrib", "struct gles_attrib"),
    ("src/backend.rs", "GlesTextureBind", "crate::backend::GlesTextureBind", "struct gles_texture_bind"),
    ("src/backend.rs", "GlesDrawTarget", "crate::backend::GlesDrawTarget", "struct gles_draw_target"),
    ("src/backend.rs", "GlesDraw", "crate::backend::GlesDraw", "struct gles_draw"),
    ("src/backend.rs", "YakoglBackend", "crate::backend::YakoglBackend", "struct yakogl_backend"),
    ("src/gl/context.rs", "GlBuffer", "crate::gl::context::GlBuffer", "struct gl_buffer"),
    ("src/gl/context.rs", "GlTexture", "crate::gl::context::GlTexture", "struct gl_texture"),
    ("src/gl/context.rs", "GlShader", "crate::gl::context::GlShader", "struct gl_shader"),
    ("src/gl/context.rs", "GlProgram", "crate::gl::context::GlProgram", "struct gl_program"),
    ("src/gl/context.rs", "GlRenderbuffer", "crate::gl::context::GlRenderbuffer", "struct gl_renderbuffer"),
    ("src/gl/context.rs", "GlAttachment", "crate::gl::context::GlAttachment", "struct gl_attachment"),
    ("src/gl/context.rs", "GlFramebuffer", "crate::gl::context::GlFramebuffer", "struct gl_framebuffer"),
    ("src/gl/context.rs", "GlVertexAttrib", "crate::gl::context::GlVertexAttrib", "struct gl_vertex_attrib"),
    ("src/gl/context.rs", "GlVao", "crate::gl::context::GlVao", "struct gl_vao"),
    ("src/gl/context.rs", "GlObjects", "crate::gl::context::GlObjects", "struct gl_objects"),
    ("src/gl/context.rs", "GlContext", "crate::gl::context::GlContext", "struct gl_context"),
    ("src/vivante/kick.rs", "GcKick", "crate::vivante::kick::GcKick", "struct gc_kick"),
    ("src/v3d/kick.rs", "V3dKick", "crate::v3d::kick::V3dKick", "struct v3d_kick"),
];
const INCLUDES: &[&str] = &[
    "\"gl_internal.h\"",
    "\"pvr/pvr_backend.h\"",
    "\"vivante/gc_kick.h\"",
    "\"v3d/v3d_kick.h\"",
    "\"osal_internal.h\"",
    "\"pvr_render.h\"",
];

/// One GPU a build can be for: its name, its C define and its cargo feature
/// (`None` for the PowerVR, the build with none of the others).
pub struct Gpu {
    pub name: &'static str,
    pub define: Option<&'static str>,
    pub feature: Option<&'static str>,
}

/// Every GPU, in the order the pins are written.
pub const GPUS: &[Gpu] = &[
    Gpu { name: "powervr", define: None, feature: None },
    Gpu { name: "vivante", define: Some("YAKOGL_GPU_VIVANTE"), feature: Some("gpu-vivante") },
    Gpu { name: "v3d", define: Some("YAKOGL_GPU_V3D"), feature: Some("gpu-v3d") },
];

impl Gpu {
    /// The Rust `cfg` predicate that selects this GPU.
    fn rust_cfg(&self) -> String {
        match self.feature {
            Some(f) => format!("feature = \"{f}\""),
            None => {
                let others: Vec<String> =
                    GPUS.iter().filter_map(|g| g.feature).map(|f| format!("feature = \"{f}\"")).collect();
                format!("not(any({}))", others.join(", "))
            }
        }
    }

    /// The C preprocessor condition that selects this GPU.
    fn c_cond(&self) -> String {
        match self.define {
            Some(d) => format!("defined({d})"),
            None => {
                GPUS.iter().filter_map(|g| g.define).map(|d| format!("!defined({d})")).collect::<Vec<_>>().join(" && ")
            }
        }
    }
}

/// The GPU whose build alone has a backend module's structs (`crate::pvr::`
/// and so on), or `None` for a struct every build has.
fn module_gpu(rpath: &str) -> Option<&'static Gpu> {
    let name = match rpath.split("::").nth(1) {
        Some("pvr") => "powervr",
        Some("vivante") => "vivante",
        Some("v3d") => "v3d",
        _ => return None,
    };
    GPUS.iter().find(|g| g.name == name)
}

/// One pinned number: (Rust path, C struct, None for the size or
/// Some((Rust field, C field)) for an offset).
type Row = (&'static str, &'static str, Option<(String, String)>);

/// One `pub` field of a Rust struct: its name, and the cargo feature of the
/// one GPU whose build alone has it (`None`: every build has it).
pub struct Field {
    pub name: String,
    pub only_feature: Option<&'static str>,
}

/// The `pub` fields of a Rust struct, in declaration order. A field carrying
/// `#[cfg(feature = "<one GPU's feature>")]` is marked rather than skipped;
/// a field behind any other `cfg` is refused, because no probe could say
/// which build it belongs to.
pub fn rust_fields(root: &str, path: &str, name: &str) -> R<Vec<Field>> {
    let text = py::read_text(&py::join(root, path))?;
    let Some(m) = re(&format!(r"(?s)pub struct {} \{{\n(.*?)\n\}}", py::esc(name))).captures(&text) else {
        return Err(Error::Exit(format!("no struct {name} in {path}")));
    };
    let body = g(&m, 1);
    let mut out = Vec::new();
    let mut gated: Option<&'static str> = None;
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("#[cfg") {
            let Some(f) = GPUS.iter().filter_map(|g| g.feature).find(|f| t == format!("#[cfg(feature = \"{f}\")]"))
            else {
                return Err(Error::Exit(format!("{path}: a field of {name} is behind `{t}`, not one GPU's feature")));
            };
            gated = Some(f);
            continue;
        }
        if let Some(c) = re(r"^pub (\w+):").captures(t) {
            out.push(Field { name: g(&c, 1).to_string(), only_feature: gated });
            gated = None;
        } else if !t.is_empty() && !t.starts_with("//") && !t.starts_with("/*") && !t.starts_with('*') {
            gated = None;
        }
    }
    Ok(out)
}

/// Every row in declaration order, each with the feature of the one GPU whose
/// build alone has it.
fn all_rows(root: &str) -> R<Vec<(Row, Option<&'static str>)>> {
    let mut rows = Vec::new();
    for (path, rname, rpath, cname) in STRUCTS {
        rows.push(((*rpath, *cname, None), None));
        for f in rust_fields(root, path, rname)? {
            // type_ -> type, impl_ -> impl, priv_ -> priv
            let cf = f.name.strip_suffix('_').unwrap_or(&f.name).to_string();
            rows.push(((*rpath, *cname, Some((f.name, cf))), f.only_feature));
        }
    }
    Ok(rows)
}

/// Measure, with the host C compiler, every row that exists in `gpu`'s build;
/// `None` for the rows that do not.
fn probe(root: &str, drv: &str, gpu: &Gpu, rows: &[(Row, Option<&'static str>)]) -> R<Vec<Option<u64>>> {
    let mut prog: Vec<String> = vec!["#include <stdio.h>".into(), "#include <stddef.h>".into()];
    prog.extend(INCLUDES.iter().map(|i| format!("#include {i}")));
    prog.push("int main(void) {".into());
    let present: Vec<bool> = rows.iter().map(|(_, only)| only.is_none() || *only == gpu.feature).collect();
    for ((row, _), _) in rows.iter().zip(&present).filter(|(_, p)| **p) {
        match &row.2 {
            None => prog.push(format!("printf(\"%zu\\n\", sizeof({}));", row.1)),
            Some((_, cf)) => prog.push(format!("printf(\"%zu\\n\", offsetof({}, {cf}));", row.1)),
        }
    }
    prog.push("return 0; }".into());
    let td = tempfile::tempdir().map_err(|e| Error::Fatal(format!("tempdir: {e}")))?;
    let src = td.path().join("m.c");
    let exe = td.path().join("m");
    std::fs::write(&src, prog.join("\n")).map_err(|e| Error::Fatal(format!("write: {e}")))?;
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut cmd = Command::new(&cc);
    for d in ["include", "src", "src/osal"] {
        cmd.arg(format!("-I{}", py::join(root, d)));
    }
    if let Some(d) = gpu.define {
        cmd.arg(format!("-D{d}"));
    }
    cmd.arg(format!("-I{drv}")).arg(&src).arg("-o").arg(&exe);
    let st = cmd.status().map_err(|e| Error::Fatal(format!("{cc}: {e}")))?;
    if !st.success() {
        return Err(Error::Fatal(format!("{cc} failed for the {} build ({st})", gpu.name)));
    }
    let out = Command::new(&exe).output().map_err(|e| Error::Fatal(format!("{}: {e}", exe.display())))?;
    if !out.status.success() {
        return Err(Error::Fatal(format!("layout probe failed for the {} build ({})", gpu.name, out.status)));
    }
    let nums: Vec<u64> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().parse().ok())
        .collect::<Option<_>>()
        .ok_or_else(|| Error::Fatal("unparsable layout probe output".into()))?;
    let want = present.iter().filter(|p| **p).count();
    if nums.len() != want {
        return Err(Error::Fatal(format!("the {} probe printed {} numbers for {want} rows", gpu.name, nums.len())));
    }
    let mut it = nums.into_iter();
    Ok(present.iter().map(|p| if *p { it.next() } else { None }).collect())
}

/// One assertion, in both languages, behind `gate` (a C condition and a Rust
/// cfg predicate, or none). A backend module's struct is not in the other
/// GPUs' builds at all, so its Rust pin always carries that module's cfg (the
/// C header defines it either way).
fn pins(row: &Row, n: u64, gate: Option<&Gpu>) -> (String, String) {
    let gate = module_gpu(row.0).or(gate);
    let cfg = gate.map_or(String::new(), |g| format!("#[cfg({})] ", g.rust_cfg()));
    match &row.2 {
        None => (
            format!("_Static_assert(sizeof({}) == {n}, \"{}\");", row.1, row.1),
            format!("{cfg}const _: () = assert!(size_of::<{}>() == {n});", row.0),
        ),
        Some((rf, cf)) => (
            format!("_Static_assert(offsetof({}, {cf}) == {n}, \"{}.{cf}\");", row.1, row.1),
            format!("{cfg}const _: () = assert!(offset_of!({}, {rf}) == {n});", row.0),
        ),
    }
}

pub fn main(argv: &[String]) -> R<()> {
    if argv.len() != 1 {
        return Err(Error::Usage(USAGE.into()));
    }
    let root = crate::repo_root();
    let drv = &argv[0];
    let rows = all_rows(&root)?;
    let nums: Vec<Vec<Option<u64>>> = GPUS.iter().map(|gpu| probe(&root, drv, gpu, &rows)).collect::<R<_>>()?;

    let mut c: Vec<String> = [
        "/* SPDX-License-Identifier: Apache-2.0 */",
        "/* GENERATED by `yakogl-tools abi-layout`. DO NOT EDIT.",
        " *",
        " * No code: pins the layout of every struct the C headers share with the Rust",
        " * crate (src/abi_layout.rs asserts the same numbers on the Rust side). If an",
        " * assertion fails, a header and its #[repr(C)] mirror have drifted apart: fix",
        " * them together and re-run the tool. A number that depends on which GPU the",
        " * build is for sits behind that GPU's define (YAKOGL_GPU_VIVANTE,",
        " * YAKOGL_GPU_V3D; the PowerVR is neither). LP64 targets only. */",
        "#include <stddef.h>",
    ]
    .map(String::from)
    .to_vec();
    c.extend(INCLUDES.iter().map(|i| format!("#include {i}")));
    c.push(String::new());
    c.push("#if defined(__LP64__)".into());
    let mut r: Vec<String> = [
        "// SPDX-License-Identifier: Apache-2.0",
        "// GENERATED by `yakogl-tools abi-layout`. DO NOT EDIT.",
        "//! Pins the layout of every `#[repr(C)]` struct shared with the C headers",
        "//! (`src/abi_layout_check.c` asserts the same numbers on the C side). A",
        "//! number that depends on which GPU the build is for is pinned once per GPU.",
        "#![cfg(target_pointer_width = \"64\")]",
        "#![cfg_attr(rustfmt, rustfmt_skip)]",
        "",
        "use core::mem::{offset_of, size_of};",
        "",
    ]
    .map(String::from)
    .to_vec();

    let mut gpu_specific = 0usize;
    for (k, (row, _)) in rows.iter().enumerate() {
        let per_gpu: Vec<(&Gpu, u64)> = GPUS.iter().zip(&nums).filter_map(|(g, n)| n[k].map(|v| (g, v))).collect();
        let every_build = per_gpu.len() == GPUS.len();
        let agree = per_gpu.windows(2).all(|w| w[0].1 == w[1].1);
        if every_build && agree {
            let (cv, rv) = pins(row, per_gpu[0].1, None);
            c.push(cv);
            r.push(rv);
            continue;
        }
        /* one pin per build that has the row, each behind its GPU's gate */
        for (gpu, n) in &per_gpu {
            let (cv, rv) = pins(row, *n, Some(gpu));
            c.push(format!("#if {}", gpu.c_cond()));
            c.push(cv);
            c.push("#endif".into());
            r.push(rv);
        }
        gpu_specific += 1;
    }
    c.push("#endif".into());
    c.push(String::new());
    r.push(String::new());
    py::write_text(&py::join(&root, "src/abi_layout_check.c"), &c.join("\n"))?;
    py::write_text(&py::join(&root, "src/abi_layout.rs"), &r.join("\n"))?;
    eprintln!(
        "abi-layout: {} structs, {} fields pinned ({gpu_specific} of them per GPU, {} GPUs)",
        STRUCTS.len(),
        rows.len() - STRUCTS.len(),
        GPUS.len()
    );
    Ok(())
}
