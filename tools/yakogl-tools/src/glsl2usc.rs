// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools glsl2usc`: GLSL-ES -> PowerVR USC blob table generator.
//!
//! For each input GLSL shader (*.vert / *.frag) it runs
//!     glslangValidator -V -S <stage> in -> SPIR-V
//!     pco_shc <vs|fs> in.spv           -> USC dwords (stdout) + metadata (stderr)
//! parses pco_shc's metadata, computes the SHA-256 of the GLSL source text,
//! and emits `tests/reference/blobs.rs`, the table the offline shader backend
//! (src/shaderc/offline.rs) looks shaders up in by source. A port of
//! upstream's tools/glsl2usc.py (7e76640) that produces entry-for-entry the
//! same table; see tools/README.md.
//!
//! A shader pco_shc cannot compile can supply a `<file>.prebuilt.json`
//! sidecar with known-good USC dwords + metadata.
//!
//! Environment: PCO_SHC (pco_shc binary), GLSLANG (glslangValidator),
//! GLSL2USC_KEEP (keep intermediates), GLSL2USC_CACHE (compiler output
//! cache directory), GLSL2USC_UNROLL (unroll uniform-array loops),
//! GLSL2USC_OPT (run glslang's SPIR-V optimiser before pco_shc).

use crate::emit;
use crate::es100;
use crate::glsl::{self, exit, GlNames, Uniform, NO_BLOCK};
use crate::list::{self, SpecError, COLORMASK_BITS};
use crate::py::{self, g, pint, re};
use crate::{Error, R};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Read;
use std::process::{Command, Stdio};
use yakogl::shaderc as abi;

/// SHC_NO_META: a shader that needs no ISP-feedback meta word.
pub const SHC_NO_META: i64 = abi::SHC_NO_META as i64;
/// Image 4 + image meta 4 + sampler 4 + sampler meta 4 + gather 4.
const COMBINED_IMG_SMP_DWORDS: i64 = 20;

/* The variant spec syntax (FACTORS[:MASK][@RT]) is parsed by the source
 * list's reader, tools/precompile/src/list.rs, so that the list's `blend`
 * and `for` lines and this tool's --blend/--for read it one way; the
 * wrappers below keep this tool's messages. */

/// A spec error in this tool's words (pinned by `tests::blend_keys`).
fn spec_error(spec: &str, e: SpecError) -> Error {
    Error::Exit(match e {
        SpecError::RtClass { rt } => {
            format!("glsl2usc: --blend '{spec}': render target class '{rt}' is not one of {}", list::rt_class_names())
        }
        SpecError::Mask { mask } => format!("glsl2usc: --blend mask '{mask}': channels are r g b a"),
        SpecError::TooFewFactors => "glsl2usc: --blend needs at least srgb,drgb".into(),
        SpecError::Unknown(e) => format!("glsl2usc: unknown blend factor/equation {}", py::repr(&e)),
        SpecError::PlainReplace => format!("glsl2usc: --blend {spec} is the plain replace the base shader already is"),
        SpecError::IntBlend => format!("glsl2usc: --blend '{spec}': an integer render target takes `replace` only"),
    })
}

/// Where `pco_shc` has been found, in order. The first that EXISTS wins.
///
/// It used to be one hardcoded path into a sibling worktree, and when that
/// worktree was deleted the tool reported "no such file" against a path
/// nobody had looked at in weeks -- which I then read as "the compiler is
/// gone from this host" and reported as a blocker. It was not gone; it was
/// at the second of these. A list that is tried, with every candidate named
/// when none is found, cannot produce that mistake again.
///
/// The third is the laptop's, and it is `build-new` on purpose: that
/// checkout's `build` compiles MRT shaders to nothing and `build2` asserts.
const PCO_SHC_CANDIDATES: [&str; 3] = [
    "../scratchpad/mesa-build/mesa-26.2.1/build/src/imagination/vulkan/pco_shc",
    "~/pvr-lab/mesa-build/mesa-26.2.1/build/src/imagination/vulkan/pco_shc",
    "~/com/github/signal-slot/yakogl-in-rust/work/pco-shc/mesa/build-new/src/imagination/vulkan/pco_shc",
];

/// Expand a leading `~` against `$HOME`; other paths are taken as given.
fn untilde(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => p.to_string(),
    }
}

/// Every candidate, resolved, for the message when none of them is there.
pub fn pco_shc_candidates() -> Vec<String> {
    PCO_SHC_CANDIDATES
        .iter()
        .map(|c| if c.starts_with("~/") { untilde(c) } else { py::abspath(&py::join(&crate::repo_root(), c)) })
        .collect()
}

fn default_pco_shc() -> String {
    let all = pco_shc_candidates();
    all.iter().find(|p| py::exists(p)).cloned().unwrap_or_else(|| all[0].clone())
}

pub fn pco_shc_path() -> String {
    std::env::var("PCO_SHC").unwrap_or_else(|_| default_pco_shc())
}

/// `GLSL2USC_OPT`: run glslang's SPIR-V optimiser before pco_shc (unset,
/// empty and "0" are off, as in the Python).
fn opt_pass() -> bool {
    py::env_set("GLSL2USC_OPT").is_some_and(|v| v != "0")
}

fn glslang_path() -> String {
    std::env::var("GLSLANG").unwrap_or_else(|_| "glslangValidator".into())
}

/// Any spelling of a stage as this file's short name (`vs` / `fs`).
pub fn stage_short(stage: &str) -> R<&'static str> {
    match es100::glslang_stage(stage)? {
        "vert" => Ok("vs"),
        _ => Ok("fs"),
    }
}

pub fn stage_of(path: &str) -> R<&'static str> {
    if path.ends_with(".vert") {
        Ok("vs")
    } else if path.ends_with(".frag") {
        Ok("fs")
    } else {
        exit!("glsl2usc: {path}: expected a .vert or .frag file")
    }
}

/// 'BLENDSPEC[@rt]' -> (blendspec, rt).
pub fn split_rt(spec: &str) -> R<(&str, &str)> {
    list::split_rt(spec).map_err(|e| spec_error(spec, e))
}

/// 'FACTORS[:MASK][@rt]' -> (factors, mask bits).
pub fn split_variant_spec(spec: &str) -> R<(String, u32)> {
    list::split_variant_spec(spec).map_err(|e| spec_error(spec, e))
}

/// Variant spec -> the blend key (SHC_BLEND_KEY | MASK_OFF | RT).
pub fn blend_key(spec: &str) -> R<u32> {
    list::blend_key(spec).map_err(|e| spec_error(spec, e))
}

/// The environment the compiler runs with for a variant (PCO_SHC_OUT,
/// PCO_SHC_BLEND, PCO_SHC_COLORMASK; None = removed).
pub fn variant_env(blend_spec: &str) -> R<[(&'static str, Option<String>); 3]> {
    let mut out = [("PCO_SHC_OUT", None), ("PCO_SHC_BLEND", None), ("PCO_SHC_COLORMASK", None)];
    if !blend_spec.is_empty() {
        let rt = split_rt(blend_spec)?.1;
        if !rt.is_empty() && rt != "rgba8" {
            out[0].1 = Some(rt.to_string()); // the colour output packed for a float target
        }
        let (factors, mask_bits) = split_variant_spec(blend_spec)?;
        if factors != "one,zero" {
            out[1].1 = Some(factors);
        }
        out[2].1 = Some(COLORMASK_BITS.iter().filter(|(_, b)| mask_bits & b != 0).map(|(c, _)| *c).collect());
    }
    Ok(out)
}

/* ---- metadata model ---------------------------------------------------------- */

#[derive(Clone, Debug, Default)]
pub struct Varying {
    pub location: i64,
    pub components: i64,
    pub interp: i64,
    pub vtxout_off: i64,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Attrib {
    pub location: i64,
    pub components: i64,
    pub vi_reg: i64,
    pub format: i64,
    pub name: String,
    pub gl_type: i64,
}

#[derive(Clone, Debug)]
pub struct Desc {
    pub set: i64,
    pub binding: i64,
    pub sh_start: i64,
    pub count: i64,
    pub stride: i64,
}

impl Desc {
    /// Python's repr of the metadata dict.
    pub fn repr(&self) -> String {
        format!(
            "{{'set': {}, 'binding': {}, 'sh_start': {}, 'count': {}, 'stride': {}}}",
            self.set, self.binding, self.sh_start, self.count, self.stride
        )
    }
}

/// A resolved shader ready to emit.
#[derive(Clone, Debug)]
pub struct Shader {
    pub name: String,
    pub path: String,
    pub stage: String,
    pub sha: [u8; 32],
    pub usc: Vec<u32>,
    pub temps: i64,
    pub uses_w: i64,
    pub uses_discard: i64,
    pub meta_off: i64,
    /// `gl_FrontFacing`: the shared-register offset of the driver-supplied
    /// front-face op, when the shader reads it.
    pub front_face_off: Option<i64>,
    pub num_outputs: i64,
    /// Fragment outputs that did not fit the pixel output registers and are
    /// stored to tile buffers, and the first shared register of their
    /// addresses.
    pub num_tile_buffers: i64,
    pub tile_buffer_sh: i64,
    pub uniform_dwords: i64,
    /// The prefix of the block the compiled shader reads.
    pub uniform_sh_dwords: i64,
    pub vs_output_size: i64,
    pub vs_psize_off: i64,
    pub vs_vertex_id_reg: Option<i64>,
    pub vs_instance_id_reg: Option<i64>,
    pub varyings: Vec<Varying>,
    pub attribs: Vec<Attrib>,
    pub uniforms: Vec<Uniform>,
    pub blend_spec: String,
    pub blend_key: u32,
    pub uses_fbfetch: i64,
    pub colormask: i64,
    pub blend_consts: Option<(i64, i64)>,
    pub blend_consts_off: i64,
    /// pco_shc's `uses: ... pntc=`: the fragment shader reads gl_PointCoord.
    pub uses_pntc: i64,
    /// pco_shc's `uses: z=`: the fragment shader reads gl_FragCoord.z (a
    /// Z coefficient set first, before W).
    pub uses_z: i64,
    pub descs: Vec<Desc>,
    pub note: String,
    pub source: String,
    /// `PCO_DEBUG` the row was built with (`--pco-debug`; empty: none). The
    /// table records it (emit.rs), so it says which rows pco built some
    /// other way than its default -- `no_pred_cf`: an `if` under the
    /// execution mask with a branch over each arm instead of predicated.
    pub pco_debug: String,
}

impl Shader {
    pub(crate) fn new(stage: &str) -> Shader {
        Shader {
            name: String::new(),
            path: String::new(),
            stage: stage.to_string(),
            sha: [0; 32],
            usc: Vec::new(),
            temps: 0,
            uses_w: 0,
            uses_discard: 0,
            meta_off: SHC_NO_META,
            front_face_off: None,
            num_outputs: 0,
            num_tile_buffers: 0,
            tile_buffer_sh: 0,
            uniform_dwords: 0,
            uniform_sh_dwords: 0,
            vs_output_size: 0,
            vs_psize_off: abi::SHC_NO_PSIZE as i64,
            vs_vertex_id_reg: None,
            vs_instance_id_reg: None,
            varyings: Vec::new(),
            attribs: Vec::new(),
            uniforms: Vec::new(),
            blend_spec: String::new(),
            blend_key: 0,
            uses_fbfetch: 0,
            colormask: 0xf,
            blend_consts: None,
            blend_consts_off: abi::SHC_NO_BLEND_CONSTS as i64,
            uses_pntc: 0,
            uses_z: 0,
            descs: Vec::new(),
            note: String::new(),
            source: String::new(),
            pco_debug: String::new(),
        }
    }

    pub fn sha_hex(&self) -> String {
        self.sha.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// FS varyings carry no VS-output offset in the metadata: reconstruct the
/// packed layout the VS emits (gl_Position first, then ascending location).
fn compute_fs_vtxout_offsets(varyings: &mut [Varying]) {
    let mut order: Vec<usize> = (0..varyings.len()).collect();
    order.sort_by_key(|&i| varyings[i].location);
    let mut off = 4;
    for i in order {
        varyings[i].vtxout_off = off;
        off += varyings[i].components;
    }
}

/// Parse pco_shc's stderr metadata into a Shader (minus name/sha/usc).
pub fn parse_pco_meta(meta: &str, stage: &str) -> R<Shader> {
    let mut sh = Shader::new(stage);
    let cap = |p: &str| re(p).captures(meta);
    if let Some(m) = cap(r"(?m)^temps=(\d+)") {
        sh.temps = pint(g(&m, 1))?;
    }
    for m in
        re(r"(?m)^desc\[set=(\d+) binding=(\d+)\]:\s*sh\.start=(\d+)\s+count=(\d+)\s+stride=(\d+)").captures_iter(meta)
    {
        sh.descs.push(Desc {
            set: pint(g(&m, 1))?,
            binding: pint(g(&m, 2))?,
            sh_start: pint(g(&m, 3))?,
            count: pint(g(&m, 4))?,
            stride: pint(g(&m, 5))?,
        });
    }
    if stage == "fs" {
        if let Some(m) = cap(r"(?m)^uses:.*\bw=(\d+).*\bdiscard=(\d+)") {
            sh.uses_w = pint(g(&m, 1))?;
            sh.uses_discard = pint(g(&m, 2))?;
        }
        if let Some(m) = cap(r"(?m)^uses:.*\bfbfetch=(\d+)") {
            sh.uses_fbfetch = pint(g(&m, 1))?;
        }
        /* gl_PointCoord: pco iterates it in the coefficients after W and
         * the driver must feed them (a point-sprite iterator). Dropped here
         * until 2026-09-23, so no record said so (gl_test case 83). */
        if let Some(m) = cap(r"(?m)^uses:.*\bpntc=(\d+)") {
            sh.uses_pntc = pint(g(&m, 1))?;
        }
        /* gl_FragCoord.z: pco iterates it from a Z coefficient set at
         * cf0..3, before W, and the driver must set that plane up first.
         * Dropped here until 2026-09-24, so no record said so and the 32
         * Qt particle shaders that read it read every input one set off. */
        if let Some(m) = cap(r"(?m)^uses:.*\bz=(\d+)") {
            sh.uses_z = pint(g(&m, 1))?;
        }
        if let Some(m) = cap(r"(?m)^colormask=0x([0-9a-f]+)") {
            sh.colormask = i64::from_str_radix(g(&m, 1), 16).map_err(|e| Error::Fatal(format!("colormask: {e}")))?;
        }
        if let Some(m) = cap(r"(?m)^blend_consts:\s*sh\.start=(\d+)\s+count=(\d+)") {
            sh.blend_consts = Some((pint(g(&m, 1))?, pint(g(&m, 2))?));
        }
        /* every colour output, not only the first: a G-buffer shader writes
         * several, and the count is how many output registers the end-of-tile
         * program emits from */
        let outs = re(r"(?m)^fs_out\[[^\]]*\]:\s*(?:po|tilebuf)\.start=\d+\s+count=(\d+)")
            .captures_iter(meta)
            .map(|m| pint(g(&m, 1)))
            .collect::<R<Vec<i64>>>()?;
        if !outs.is_empty() {
            sh.num_outputs = outs.iter().sum();
        }
        /* outputs that did not fit the pixel output registers live in tile
         * buffers; the driver writes their addresses to shared registers:
         *   tile_buffers (driver-supplied addresses): sh.start=N count=M stride=2 buffers=B
         */
        if let Some(m) = cap(r"(?m)^tile_buffers[^\n]*sh\.start=(\d+)\s+count=\d+\s+stride=\d+\s+buffers=(\d+)") {
            sh.tile_buffer_sh = pint(g(&m, 1))?;
            sh.num_tile_buffers = pint(g(&m, 2))?;
        }
        // push-const uniform block -> uniform_dwords (the discard meta word is not one)
        if let Some(m) = cap(r"(?m)^push_consts[^\n]*count=(\d+)\s*dwords") {
            sh.uniform_dwords = pint(g(&m, 1))?;
        }
        // full-mode (discard) shaders: the driver-supplied meta word at sh[meta_off]
        if let Some(m) = cap(r"(?m)^meta\s*\([^)]*\):\s*sh\.start=(\d+)\s+count=(\d+)") {
            sh.meta_off = pint(g(&m, 1))?;
        }
        // gl_FrontFacing: the front-face op register (driver-supplied)
        if let Some(m) = cap(r"(?m)^front_face_op:\s*sh\.start=(\d+)") {
            sh.front_face_off = Some(pint(g(&m, 1))?);
        }
        // user input varyings: fs_in[slot=32+loc VARn]: coeff.start=.. count=<dwords>
        for m in re(r"(?m)^fs_in\[slot=(\d+)\s+VARn\]:\s*coeff\.start=(\d+)\s+count=(\d+)").captures_iter(meta) {
            let slot = pint(g(&m, 1))?;
            let count_dw = pint(g(&m, 3))?;
            sh.varyings.push(Varying {
                location: slot - 32,                // VARYING_SLOT_VAR0 == 32
                components: count_dw.div_euclid(4), // one 4-dword coefficient set per component
                interp: 0,
                vtxout_off: 0,
                name: String::new(),
            });
        }
        compute_fs_vtxout_offsets(&mut sh.varyings);
    } else {
        if let Some(m) = cap(r"(?m)^vtxouts\(vs_output_size\)=(\d+)") {
            sh.vs_output_size = pint(g(&m, 1))?;
        }
        if let Some(m) = cap(r"(?m)^vs_out\[slot=12 [^\]]*\]:\s*vtxout\.start=(\d+)\s+count=1") {
            sh.vs_psize_off = pint(g(&m, 1))?;
        }
        if let Some(m) = cap(r"(?m)^sysval\[SYSTEM_VALUE_VERTEX_ID\]:\s*vtxin\.start=(\d+)") {
            sh.vs_vertex_id_reg = Some(pint(g(&m, 1))?);
        }
        if let Some(m) = cap(r"(?m)^sysval\[SYSTEM_VALUE_INSTANCE_ID\]:\s*vtxin\.start=(\d+)") {
            sh.vs_instance_id_reg = Some(pint(g(&m, 1))?);
        }
        for m in re(r"(?m)^vs_out\[slot=(\d+)\s+VARn\]:\s*vtxout\.start=(\d+)\s+count=(\d+)").captures_iter(meta) {
            sh.varyings.push(Varying {
                location: pint(g(&m, 1))? - 32,
                components: pint(g(&m, 3))?,
                interp: 0,
                vtxout_off: pint(g(&m, 2))?,
                name: String::new(),
            });
        }
        if let Some(m) = cap(r"(?m)^push_consts[^\n]*count=(\d+)\s*dwords") {
            sh.uniform_dwords = pint(g(&m, 1))?;
        }
        for m in re(r"(?m)^attrib\[loc=\d+\s+gen=(\d+)\]:\s*vtxin\.start=(\d+)\s+count=(\d+)").captures_iter(meta) {
            sh.attribs.push(Attrib {
                location: pint(g(&m, 1))?, // generic index == GLSL location
                components: pint(g(&m, 3))?,
                vi_reg: pint(g(&m, 2))?,
                format: 0, // float32
                name: String::new(),
                gl_type: 0,
            });
        }
    }
    Ok(sh)
}

/* ---- running the compiler ---------------------------------------------------- */

/// `str(FileNotFoundError)` as subprocess raises it.
/// A compiler binary that is not there.
///
/// `Error::Fatal`, never `Error::Exit`: this is the service being broken, not
/// a verdict on the shader. An `Exit` reaches `shader-check`'s caller as
/// `{"ok": false}` -- "your shader is bad" -- which on a host with no
/// `pco_shc` would call every shader invalid. `--skip-failures` must not
/// quietly drop shaders for this reason either.
fn not_found(prog: &str) -> Error {
    Error::Fatal(format!(
        "tool not found: [Errno 2] No such file or directory: {} (set PCO_SHC / GLSLANG)",
        py::repr(prog)
    ))
}

fn spawn_err(prog: &str, e: std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        not_found(prog)
    } else {
        Error::Fatal(format!("cannot run {prog}: {e}"))
    }
}

/// How a compiler process ended, for a failure message: " (exit status 1)",
/// " (killed by signal 6)". Empty when it exited normally with a code the
/// caller already knows, so a message that carries a real log is unchanged.
fn how_it_died(st: &std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (st.code(), st.signal()) {
        (_, Some(sig)) => format!(" (killed by signal {sig})"),
        (Some(code), None) => format!(" (exit status {code})"),
        (None, None) => String::new(),
    }
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn dwords(raw: &[u8]) -> Vec<u32> {
    raw.chunks(4)
        .map(|c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            u32::from_le_bytes(w)
        })
        .collect()
}

/// `int(os.stat(p).st_mtime)`: Python builds the float as sec + nsec * 1e-9.
fn stat_tool_id(p: &str) -> String {
    content_id(p)
}

/// A bare command name (`glslangValidator`, the GLSLANG default) names the
/// file the shell would run: the first match on PATH. Anything with a `/`
/// is a path and stays as it is. Without this the identity of a tool named
/// by command was `?missing` while the tool itself ran fine.
fn resolve_on_path(p: &str) -> std::path::PathBuf {
    if p.contains('/') {
        return std::path::PathBuf::from(p);
    }
    std::env::var_os("PATH")
        .map(|v| {
            std::env::split_paths(&v)
                .map(|d| d.join(p))
                .find(|f| f.is_file())
                .unwrap_or_else(|| std::path::PathBuf::from(p))
        })
        .unwrap_or_else(|| std::path::PathBuf::from(p))
}

/// A tool's identity: the first 12 hex digits of its contents' SHA-256.
///
/// It used to be `size:mtime`, which is not an identity. Two builds of
/// `pco_shc` of the same size collide if their timestamps match, and `cp -p`
/// or `touch -r` makes them match on purpose -- so a changed compiler could
/// have been served old output out of the cache. The cost of hashing the
/// binary is paid once per run and the binary is a few megabytes.
///
/// An unreadable tool is reported as `?<reason>` rather than as a constant,
/// so it cannot silently equal another unreadable tool.
fn content_id(p: &str) -> String {
    match std::fs::read(resolve_on_path(p)) {
        Ok(b) => {
            let d = Sha256::digest(&b);
            d.iter().take(6).map(|x| format!("{x:02x}")).collect()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "?missing".into(),
        Err(e) => format!("?{}", e.kind()),
    }
}

/// What built this table: the identity of every tool whose output lands in it.
///
/// Recorded in the generated table's header and compared against it on the
/// next run, because the committed table is the reference. A rebuild with a
/// different toolchain produces a DIFFERENT reference -- on the GC7000 side
/// the same experiment changed 308 of 324 existing blobs -- and that is a
/// deliberate act, not something to discover afterwards from a diff.
pub fn tool_identity() -> String {
    format!("pco_shc={} glslang={}", content_id(&pco_shc_path()), content_id(&glslang_path_public()))
}

/// `glslang_path` for callers outside this module's privacy.
pub fn glslang_path_public() -> String {
    glslang_path()
}

/// The header line the table carries and the next run checks.
pub const TOOLCHAIN_MARK: &str = "// toolchain: ";

/// Refuse to overwrite a table that a different toolchain built.
///
/// Returns the recorded identity when it differs, so the caller can say what
/// changed rather than only that something did.
fn recorded_identity(path: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| l.strip_prefix(TOOLCHAIN_MARK).map(|s| s.trim().to_string()))
}

/// Refuse to replace the reference table with one a different toolchain built.
///
/// Called BEFORE any shader is compiled. Compiling two thousand shaders and
/// then refusing to write them would be a slow way to say no, and the answer
/// does not depend on the result.
///
/// An existing table with NO recorded toolchain is refused too. That is not
/// pedantry: it is the situation this check was written for. The committed
/// table predates the mark, so nothing can confirm that this host would
/// reproduce it, and letting the first unlabelled rebuild through would
/// replace the reference with one whose provenance is simply this machine.
fn check_reference(path: &str, ident: &str) -> R<()> {
    if path.is_empty() || path == "-" || !py::exists(path) {
        return Ok(());
    }
    if py::env_set("GLSL2USC_NEW_REFERENCE").is_some_and(|v| v != "0") {
        eprintln!("glsl2usc: GLSL2USC_NEW_REFERENCE is set: writing a NEW reference table built by {ident}");
        return Ok(());
    }
    let tail = [
        "Every number measured against the committed blobs -- the twin-draw partition, the chain",
        "score, the wedge-ladder byte comparisons -- is measured against THOSE bytes and would have",
        "to be re-derived in the same commit. If that is what you mean, set GLSL2USC_NEW_REFERENCE=1",
        "and say so in the commit message.",
    ]
    .join("\n  ");
    match recorded_identity(path) {
        Some(was) if was == ident => Ok(()),
        Some(was) => Err(Error::Fatal(format!(
            "{path} was built by a different toolchain, so rebuilding it here would produce a NEW\n  reference rather than reproduce this one.\n    recorded:  {was}\n    this host: {ident}\n  {tail}"
        ))),
        None => Err(Error::Fatal(format!(
            "{path} records no toolchain, so nothing can show that this host reproduces it. It is\n  the reference until something proves otherwise, and an unlabelled rebuild would quietly\n  replace it with whatever this machine happens to produce.\n    this host: {ident}\n  {tail}"
        ))),
    }
}

/// Who is asking the front end, which decides what a failed validation means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strictness {
    /// The offline table generator. A text the GLSL ES rules reject is named
    /// and compiled anyway: the table is a cache of binaries for texts the
    /// project already ships, some of which are not GLSL ES at all (the Qt
    /// Quick 3D sweep harvested shaders that say `#version 100` and then
    /// declare a `sampler2DArray`). Refusing them would delete working rows
    /// and stop those scenes rendering, to satisfy a rule about a different
    /// code path.
    TableBuild,
    /// An application compiling a shader through `glCompileShader`. Here the
    /// GLSL ES rules are the contract: a text they reject must fail to
    /// compile, with the log, which is what dEQP-GLES2's
    /// `shaders.*.invalid.*` cases check.
    Runtime,
}

/// The GLSL ES front end on a source file: the text everything downstream
/// works on (`es100.rs` explains why there is a step here at all).
///
/// For a GLSL ES source this validates the application's own bytes under GLSL
/// ES rules and then returns the *preprocessed* text, which is what the
/// rewriting in `glsl.rs` must see: the rewriting is regular expressions, and
/// it has no business looking at a declaration inside a false `#if`. For a
/// GLSL 450 source (this project's internal shaders) there is nothing to do
/// and the bytes come back unchanged.
///
/// Whatever comes out, the table key stays the SHA-256 of the file's original
/// bytes — computed by [`resolve_shader`], not here.
pub fn frontend_text(src_path: &str, stage: &str) -> R<String> {
    frontend_text_mode(src_path, stage, Strictness::TableBuild)
}

/// [`frontend_text`] for a named caller (see [`Strictness`]).
pub fn frontend_text_mode(src_path: &str, stage: &str, how: Strictness) -> R<String> {
    /* A fragment shader is resolved once per blend variant — eighteen times
     * for the FemtoVG set — and the front end's answer depends only on the
     * file and the stage, so it is worth remembering. Two glslang runs per
     * variant would otherwise dominate a table rebuild. */
    static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> =
        std::sync::OnceLock::new();
    let memo = SEEN.get_or_init(Default::default);
    let key = format!("{how:?}\0{stage}\0{src_path}");
    if let Some(t) = memo.lock().expect("front-end memo").get(&key) {
        return Ok(t.clone());
    }
    let text = frontend_text_uncached(src_path, stage, how)?;
    memo.lock().expect("front-end memo").insert(key, text.clone());
    Ok(text)
}

fn frontend_text_uncached(src_path: &str, stage: &str, how: Strictness) -> R<String> {
    let text = py::lossy_text(&py::read_bytes(src_path)?);
    if glsl::es_version(&text)?.is_none() {
        return Ok(text);
    }
    let scratch = es100::Scratch::new()?;
    /* The validation is reported here, not enforced. At run time a text the
     * GLSL ES rules reject must fail to compile, and `shader-check` makes it
     * do so. This table is the other thing: a cache of binaries for texts the
     * project already ships, some of which are not GLSL ES at all — the Qt
     * Quick 3D sweep harvested shaders that say `#version 100` and then
     * declare a `sampler2DArray`, which no ES 1.00 compiler accepts. Refusing
     * them here would delete working entries from the table and stop those
     * scenes rendering, to satisfy a rule about a different code path. So the
     * text is named and compiled as before, and the note says what is wrong
     * with it. */
    if let Err(d) = es100::validate(&scratch, stage, &text)? {
        if how == Strictness::Runtime {
            exit!("{}", d.log.trim_end());
        }
        for line in d.log.lines().filter(|l| !l.trim().is_empty()) {
            eprintln!("glsl2usc: {}: not valid GLSL ES (compiled anyway): {}", py::basename(src_path), line);
        }
    }
    match es100::preprocess(&scratch, stage, &text)? {
        Ok(t) => Ok(t),
        /* A text the preprocessor cannot even read has no translation, so
         * this one is fatal (and `--skip-failures` still skips it). */
        Err(d) => exit!("glsl2usc: {}: the preprocessor rejects this shader:\n{}", py::basename(src_path), d.log),
    }
}

/// glslang + pco_shc in a temp dir removed afterwards unless GLSL2USC_KEEP.
pub fn run_pco(src_path: &str, stage: &str, blend_spec: &str) -> R<(Vec<u32>, String)> {
    let text = frontend_text(src_path, stage)?;
    run_pco_text(&text, src_path, stage, blend_spec)
}

/// [`run_pco`] on text the front end has already passed (so that a caller
/// which also reflects the source runs the front end once, not twice, and
/// reflects exactly the text that was compiled).
pub fn run_pco_text(text: &str, src_path: &str, stage: &str, blend_spec: &str) -> R<(Vec<u32>, String)> {
    run_pco_text_debug(text, src_path, stage, blend_spec, "")
}

/// [`run_pco_text`] with pco_shc run under `PCO_DEBUG=pco_debug` (empty: the
/// environment's own `PCO_DEBUG`, if any, as before).
pub fn run_pco_text_debug(
    text: &str,
    src_path: &str,
    stage: &str,
    blend_spec: &str,
    pco_debug: &str,
) -> R<(Vec<u32>, String)> {
    let base = std::env::temp_dir();
    let tmpd = tempfile::Builder::new()
        .prefix("glsl2usc.")
        .rand_bytes(8) // the length of mkdtemp's names: messages quote the path
        .tempdir_in(&base)
        .map_err(|e| Error::Fatal(format!("mkdtemp: {e}")))?;
    let dir = tmpd.path().to_string_lossy().into_owned();
    let r = run_pco_in(&dir, text, src_path, stage, blend_spec, pco_debug);
    if py::env_set("GLSL2USC_KEEP").is_some() {
        let _ = tmpd.keep();
    }
    r
}

/// What a cache entry is keyed by, besides the compiled text: the stage,
/// the variant, the compiler binary, the file name, the optimiser pass and
/// the `PCO_DEBUG` pco_shc runs under. The last is appended only when set, so
/// every entry made without it keeps its key.
fn cache_key_material(stage: &str, blend_spec: &str, tool_id: &str, base: &str, opt: bool, pco_debug: &str) -> String {
    let mut m = format!("{}|{}|{}|{}|opt{}|", stage, blend_spec, tool_id, base, if opt { "1" } else { "0" });
    if !pco_debug.is_empty() {
        m.push_str(&format!("pco_debug={pco_debug}|"));
    }
    m
}

fn run_pco_in(
    tmpd: &str,
    fe_text: &str,
    src_path: &str,
    stage: &str,
    blend_spec: &str,
    pco_debug: &str,
) -> R<(Vec<u32>, String)> {
    let glslang_stage = if stage == "vs" { "vert" } else { "frag" };
    let keep = py::env_set("GLSL2USC_KEEP");
    let spv = py::join(tmpd, "s.spv");
    let mut src_path = src_path.to_string();
    let mut text = fe_text.to_string();
    if let Some(es) = glsl::es_version(&text)? {
        // GLSL ES source: compile the 450 rewrite (same stage, same file name)
        text = glsl::translate(&text, stage, es)?.0;
        src_path = py::join(tmpd, py::basename(&src_path));
        py::write_text(&src_path, &text)?;
        if keep.is_some() {
            py::write_text(&keep_path_for(&src_path, ".es100to450.glsl"), &text)?;
        }
    }
    let env = variant_env(blend_spec)?;
    /* The PCO_DEBUG pco_shc runs under: --pco-debug's, else the caller's
     * environment's (inherited, as it always was). Either way it is in the
     * cache key: an entry built under one must not be served for another. */
    let pco_debug =
        if pco_debug.is_empty() { py::env_set("PCO_DEBUG").unwrap_or_default() } else { pco_debug.to_string() };
    // GLSL2USC_CACHE=<dir>: the compiler's raw output keyed by the text it
    // compiled, the stage, the variant and the compiler binary itself.
    let cache_dir = py::env_set("GLSL2USC_CACHE");
    let mut cache_key = None;
    if let Some(dir) = &cache_dir {
        let tool_id = stat_tool_id(&pco_shc_path());
        let mut h = Sha256::new();
        h.update(
            cache_key_material(stage, blend_spec, &tool_id, py::basename(&src_path), opt_pass(), &pco_debug).as_bytes(),
        );
        h.update(text.as_bytes());
        let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let key = py::join(dir, &hex);
        if py::exists(&format!("{key}.usc")) && py::exists(&format!("{key}.meta")) {
            let raw = py::read_bytes(&format!("{key}.usc"))?;
            let meta = py::read_text(&format!("{key}.meta"))?;
            return Ok((dwords(&raw), meta));
        }
        cache_key = Some(key);
    }
    // glslang reports compile errors on stdout, not stderr: one pipe for both.
    let glslang = glslang_path();
    let (mut rd, wr) = std::io::pipe().map_err(|e| Error::Fatal(format!("pipe: {e}")))?;
    let wr2 = wr.try_clone().map_err(|e| Error::Fatal(format!("pipe: {e}")))?;
    let mut cmd = Command::new(&glslang);
    /* GLSL2USC_OPT=1 runs glslang's SPIR-V optimiser (spirv-opt's -Os pass
     * set) before pco_shc sees the module: the unoptimised output carries
     * every temporary the front end made, and the USC code this board runs is
     * what the frame rate is made of. */
    if opt_pass() {
        cmd.arg("-V").arg("-Os");
    } else {
        cmd.arg("-V");
    }
    cmd.args(["-S", glslang_stage, &src_path, "-o", &spv]).stdin(Stdio::inherit()).stdout(wr).stderr(wr2);
    let mut child = cmd.spawn().map_err(|e| spawn_err(&glslang, e))?;
    drop(cmd);
    let mut merged = Vec::new();
    rd.read_to_end(&mut merged).map_err(|e| Error::Fatal(format!("read: {e}")))?;
    let st = child.wait().map_err(|e| Error::Fatal(format!("wait: {e}")))?;
    if !st.success() {
        exit!("glsl2usc: {} failed for {}:\n{}", py::basename(&glslang), src_path, lossy(&merged));
    }
    let pco = pco_shc_path();
    let mut cmd = Command::new(&pco);
    cmd.args([stage, &spv]);
    if !pco_debug.is_empty() {
        cmd.env("PCO_DEBUG", &pco_debug);
    }
    for (k, v) in &env {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    let out = cmd.stdin(Stdio::inherit()).output().map_err(|e| spawn_err(&pco, e))?;
    if !out.status.success() {
        let mut err = out.stdout.clone();
        err.extend_from_slice(&out.stderr);
        /* A compiler that dies on an assertion writes to stderr and then
         * aborts, and on the board that text arrived empty -- leaving
         * "pco_shc failed for shader.vert:" and nothing else, which says
         * neither what is wrong nor that anything crashed. Say how it died
         * when it left no words behind. */
        exit!("glsl2usc: {} failed for {}{}:\n{}", py::basename(&pco), src_path, how_it_died(&out.status), lossy(&err));
    }
    let raw = out.stdout;
    if raw.len() % 4 != 0 {
        exit!("glsl2usc: {src_path}: USC byte length not a multiple of 4");
    }
    let usc = dwords(&raw);
    let meta = lossy(&out.stderr);
    /* pco_shc's notes (an output that did not fit the pixel output registers,
     * a shader forced into full mode) are worth seeing: the rest of its
     * stderr is the metadata this parses. */
    for line in meta.lines().filter(|l| l.starts_with("note:")) {
        eprintln!("glsl2usc: {}: {}", py::basename(&src_path), line);
    }
    if let (Some(dir), Some(key)) = (&cache_dir, &cache_key) {
        std::fs::create_dir_all(dir).map_err(|e| Error::Fatal(format!("mkdir {dir}: {e}")))?;
        std::fs::write(format!("{key}.usc"), &raw).map_err(|e| Error::Fatal(format!("cache: {e}")))?;
        py::write_text(&format!("{key}.meta"), &meta)?;
    }
    if keep.is_some() {
        py::write_text(&keep_path_for(&src_path, ".meta.txt"), &meta)?;
    }
    Ok((usc, meta))
}

/// GLSL2USC_KEEP=<dir>: where a debug copy for src_path goes.
fn keep_path_for(src_path: &str, suffix: &str) -> String {
    let d = match py::env_set("GLSL2USC_KEEP") {
        Some(k) if py::isdir(&k) => k,
        _ => py::dirname(src_path).to_string(),
    };
    py::join(&d, &format!("{}{}", py::basename(src_path), suffix))
}

/* ---- prebuilt sidecars --------------------------------------------------------- */

fn jget<'a>(j: &'a serde_json::Map<String, serde_json::Value>, k: &str) -> Option<&'a serde_json::Value> {
    j.get(k)
}

fn jint(j: &serde_json::Map<String, serde_json::Value>, k: &str, dflt: i64) -> R<i64> {
    match jget(j, k) {
        Some(v) => to_i64(py::py_int_json(v, false)?),
        None => Ok(dflt),
    }
}

fn to_i64(v: i128) -> R<i64> {
    i64::try_from(v).map_err(|_| Error::Fatal(format!("integer {v} out of range")))
}

fn load_prebuilt(src_path: &str, sidecar: &str) -> R<Shader> {
    let text = py::read_text(sidecar)?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| Error::Fatal(format!("{sidecar}: invalid JSON: {e}")))?;
    let serde_json::Value::Object(j) = v else {
        return Err(Error::Fatal(format!("{sidecar}: not a JSON object")));
    };
    let ext_stage = stage_of(src_path)?;
    let stage = match jget(&j, "stage") {
        Some(serde_json::Value::String(s)) if s == "vs" || s == "fs" => s.clone(),
        Some(other) => return Err(Error::Fatal(format!("{sidecar}: stage {other} is not \"vs\" or \"fs\""))),
        None => ext_stage.to_string(),
    };
    let mut sh = Shader::new(&stage);
    sh.name = py::basename(src_path).to_string();
    sh.temps = jint(&j, "temps", 0)?;
    sh.uses_w = jint(&j, "uses_w", 0)?;
    sh.uses_discard = jint(&j, "uses_discard", 0)?;
    sh.uses_pntc = jint(&j, "uses_pntc", 0)?;
    sh.uses_z = jint(&j, "uses_z", 0)?;
    sh.num_outputs = jint(&j, "num_outputs", 0)?;
    sh.num_tile_buffers = jint(&j, "num_tile_buffers", 0)?;
    sh.tile_buffer_sh = jint(&j, "tile_buffer_sh", 0)?;
    sh.uniform_dwords = jint(&j, "uniform_dwords", 0)?;
    sh.uniform_sh_dwords = jint(&j, "uniform_sh_dwords", sh.uniform_dwords)?;
    sh.vs_output_size = jint(&j, "vs_output_size", 0)?;
    sh.vs_psize_off = jint(&j, "vs_psize_off", 0xff)?;
    let opt = |k: &str| -> R<Option<i64>> {
        match jget(&j, k) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(v) => Ok(Some(to_i64(py::py_int_json(v, false)?)?)),
        }
    };
    sh.vs_vertex_id_reg = opt("vs_vertex_id_reg")?;
    sh.vs_instance_id_reg = opt("vs_instance_id_reg")?;
    sh.note = match jget(&j, "note") {
        Some(v) if py::truthy_json(v) => py::py_str_json(v),
        _ => String::new(),
    };
    let list = |k: &str| -> R<Vec<serde_json::Map<String, serde_json::Value>>> {
        match jget(&j, k) {
            None => Ok(Vec::new()),
            Some(serde_json::Value::Array(a)) => a
                .iter()
                .map(|x| match x {
                    serde_json::Value::Object(o) => Ok(o.clone()),
                    _ => Err(Error::Fatal(format!("{sidecar}: {k} entries must be objects"))),
                })
                .collect(),
            Some(_) => Err(Error::Fatal(format!("{sidecar}: {k} must be a list"))),
        }
    };
    let req = |o: &serde_json::Map<String, serde_json::Value>, k: &str| -> R<i64> {
        match o.get(k) {
            Some(v) => to_i64(py::py_int_json(v, false)?),
            None => Err(Error::Fatal(format!("KeyError: {}", py::repr(k)))),
        }
    };
    for v in list("varyings")? {
        sh.varyings.push(Varying {
            location: req(&v, "location")?,
            components: req(&v, "components")?,
            interp: jint(&v, "interp", 0)?,
            vtxout_off: jint(&v, "vtxout_off", 0)?,
            name: String::new(),
        });
    }
    for a in list("attribs")? {
        sh.attribs.push(Attrib {
            location: req(&a, "location")?,
            components: req(&a, "components")?,
            vi_reg: jint(&a, "vi_reg", 0)?,
            format: jint(&a, "format", 0)?,
            name: a.get("name").map(py::py_str_json).unwrap_or_default(),
            gl_type: jint(&a, "gl_type", 0)?,
        });
    }
    let usc = match jget(&j, "usc") {
        Some(serde_json::Value::Array(a)) => a,
        Some(_) => return Err(Error::Fatal(format!("{sidecar}: usc must be a list"))),
        None => return Err(Error::Fatal("KeyError: 'usc'".into())),
    };
    for x in usc {
        let v = match x {
            serde_json::Value::String(_) => py::py_int_json(x, true)?,
            _ => py::py_int_json(x, false)?,
        };
        sh.usc.push(u32::try_from(v).map_err(|_| Error::Fatal(format!("{sidecar}: usc dword {v} out of range")))?);
    }
    Ok(sh)
}

/* ---- resolving one shader ------------------------------------------------------ */

/// The descriptors the compiler emitted must be the used samplers, packed
/// contiguously at sh0.. in binding order (20 dwords each).
fn check_descriptors(sh: &Shader) -> R<()> {
    if sh.descs.is_empty() {
        return Ok(());
    }
    let mut descs = sh.descs.clone();
    descs.sort_by_key(|d| d.binding);
    for (i, d) in descs.iter().enumerate() {
        let i = i as i64;
        if d.stride != COMBINED_IMG_SMP_DWORDS || d.sh_start != i * COMBINED_IMG_SMP_DWORDS {
            exit!(
                "glsl2usc: {}: used sampler binding {} at sh{} stride {}; expected the {}-th used sampler at sh{} stride {}",
                sh.name,
                d.binding,
                d.sh_start,
                d.stride,
                i,
                i * COMBINED_IMG_SMP_DWORDS,
                COMBINED_IMG_SMP_DWORDS
            );
        }
    }
    let mut samplers: Vec<i64> = sh.uniforms.iter().filter(|u| u.dword_off == NO_BLOCK).map(|u| u.binding).collect();
    samplers.sort();
    let got: Vec<i64> = descs.iter().map(|d| d.binding).collect();
    if got != samplers {
        let l = |v: &[i64]| format!("[{}]", v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", "));
        exit!("glsl2usc: {}: compiler sampler bindings {} != reflected {}", sh.name, l(&got), l(&samplers));
    }
    Ok(())
}

/// The reflection a shader's GLSL gives (used by resolve and the debug dump).
pub struct Reflection {
    pub es: Option<i64>,
    pub glsl: String,
    pub gl_names: GlNames,
}

pub fn reflect_source(glsl_bytes: &[u8], stage: &str) -> R<Reflection> {
    let mut glsl = py::lossy_text(glsl_bytes);
    let mut gl_names = GlNames::default();
    let es = glsl::es_version(&glsl)?;
    if let Some(es) = es {
        let (t, n) = glsl::translate(&glsl, stage, es)?;
        glsl = t;
        gl_names = n;
    }
    Ok(Reflection { es, glsl, gl_names })
}

/// Build one table entry: Ok(Err(reason)) when the variant is skipped.
/// `how` names the caller (see [`Strictness`]); pco_shc runs under
/// `PCO_DEBUG=pco_debug` (`--pco-debug`; empty: none), and a prebuilt
/// sidecar, which is not compiled, is refused with one.
pub fn resolve_shader(
    src_path: &str,
    blend_spec: &str,
    how: Strictness,
    pco_debug: &str,
) -> R<Result<(Shader, String), String>> {
    let src_bytes = py::read_bytes(src_path)?;
    let name = py::basename(src_path).to_string();
    let stage = stage_of(src_path)?;
    let sidecar = format!("{src_path}.prebuilt.json");
    let has_sidecar = py::exists(&sidecar);
    let (mut sh, origin, comp_path, fe_text);
    if has_sidecar {
        if !blend_spec.is_empty() {
            return Ok(Err("prebuilt: no blend variants".into()));
        }
        if !pco_debug.is_empty() {
            exit!(
                "glsl2usc: {}: --pco-debug {pco_debug} on a prebuilt sidecar, which is not compiled",
                py::basename(src_path)
            );
        }
        sh = load_prebuilt(src_path, &sidecar)?;
        origin = format!("prebuilt ({})", py::basename(&sidecar));
        comp_path = src_path.to_string();
        fe_text = frontend_text_mode(&comp_path, stage, how)?;
    } else {
        // <name>.<ext>.override: compile THIS text in place of the source,
        // still keyed by the application's original bytes.
        let over = format!("{src_path}.override");
        comp_path = if py::exists(&over) { over } else { src_path.to_string() };
        /* The GLSL ES front end runs once here, and both the code generator
         * and the reflection below work on its result: a reflection of
         * different text than was compiled would disagree with the binary. */
        fe_text = frontend_text_mode(&comp_path, stage, how)?;
        let (usc, meta) = run_pco_text_debug(&fe_text, &comp_path, stage, blend_spec, pco_debug)?;
        sh = parse_pco_meta(&meta, stage)?;
        sh.usc = usc;
        sh.pco_debug = pco_debug.to_string();
        origin = format!(
            "pco_shc{}{}",
            if comp_path != src_path { " override" } else { "" },
            if pco_debug.is_empty() { String::new() } else { format!(" PCO_DEBUG={pco_debug}") }
        );
    }
    sh.name = name.clone();
    sh.path = py::abspath(src_path);
    sh.sha = Sha256::digest(&src_bytes).into();
    sh.source =
        String::from_utf8(src_bytes).map_err(|e| Error::Fatal(format!("{src_path}: source is not UTF-8: {e}")))?;
    let refl = reflect_source(fe_text.as_bytes(), stage)?;
    let (uniforms, block_dw) = glsl::reflect_uniforms(&refl.glsl, &refl.gl_names)?;
    sh.uniforms = uniforms;
    let vnames = glsl::reflect_varyings(&refl.glsl, stage)?;
    let flat = glsl::reflect_flat_varyings(&refl.glsl, stage)?;
    for v in sh.varyings.iter_mut() {
        v.name = vnames.get(&v.location).cloned().unwrap_or_default();
        if flat.contains(&v.location) {
            v.interp = 2; // shc_varying.interp: 2 = flat
        }
    }
    if stage == "vs" {
        let names = glsl::reflect_attribs(&refl.glsl)?;
        for a in sh.attribs.iter_mut() {
            let Some((n, t)) = names.get(&a.location) else {
                exit!("glsl2usc: {}: attribute at location {} has no GLSL declaration", name, a.location);
            };
            a.name = n.clone();
            a.gl_type = *t;
        }
    }
    // Drop declared-but-unused samplers: pco_shc emits a descriptor only for
    // the samplers a variant samples.
    let used: HashSet<i64> = sh.descs.iter().map(|d| d.binding).collect();
    sh.uniforms.retain(|u| u.dword_off != NO_BLOCK || used.contains(&u.binding));
    if sh.uniform_dwords > block_dw {
        exit!(
            "glsl2usc: {}: the compiler loads {} uniform dwords but the std430 block is only {}",
            name,
            sh.uniform_dwords,
            block_dw
        );
    }
    // The compiler may load only a prefix of the block; the runtime uploads
    // the full std430 block, except in full (discard) mode, or when the shader
    // reads gl_FrontFacing, where a driver-supplied word sits right after the
    // loaded prefix.
    let compiler_block = sh.uniform_dwords;
    let trimmed = compiler_block < block_dw;
    // What the USC actually reads: the driver need only DMA this prefix into
    // the shared registers, however much of the block the application writes.
    sh.uniform_sh_dwords = compiler_block;
    sh.uniform_dwords =
        if sh.meta_off != SHC_NO_META || sh.front_face_off.is_some() { compiler_block } else { block_dw };
    check_descriptors(&sh)?;
    if !blend_spec.is_empty() {
        sh.blend_spec = blend_spec.to_string();
        sh.blend_key = blend_key(blend_spec)?;
        let want = split_variant_spec(blend_spec)?.1;
        if sh.colormask != want as i64 {
            exit!(
                "glsl2usc: {}: compiler colour mask {} != requested for '{}'",
                name,
                py::hex(sh.colormask as i128, true),
                blend_spec
            );
        }
    }
    if let Some((start, count)) = sh.blend_consts {
        // A CONSTANT_* blend appends the blend colour right after the loaded
        // uniform block, which cannot coexist with a trimmed block.
        if trimmed {
            return Ok(Err("CONSTANT_* blend not used with a trimmed uniform block".into()));
        }
        let expect = sh.descs.len() as i64 * COMBINED_IMG_SMP_DWORDS + sh.uniform_dwords;
        if start != expect || count != 4 {
            exit!("glsl2usc: {}: blend constants at sh{} x{}, expected sh{} x4", name, start, count, expect);
        }
        sh.blend_consts_off = sh.uniform_dwords;
        sh.uniform_dwords += count;
        sh.uniform_sh_dwords += count;
    }
    Ok(Ok((sh, origin)))
}

/* ---- the command -------------------------------------------------------------- */

/// `--pco-debug GLOB=FLAGS` items as (glob, flags): FLAGS a comma list of
/// Mesa pco debug option names (lower case, digits and `_`).
fn parse_pco_debug(items: &[String]) -> R<Vec<(String, String)>> {
    let mut out = Vec::new();
    for item in items {
        let (glob, flags) = py::partition(item, "=");
        let ok = !glob.is_empty()
            && !flags.is_empty()
            && flags
                .split(',')
                .all(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
        if !ok {
            exit!("glsl2usc: --pco-debug expects GLOB=FLAGS (e.g. '*/offarm/*.branch.frag=no_pred_cf'), got '{item}'");
        }
        out.push((glob.to_string(), flags.to_string()));
    }
    Ok(out)
}

/// The `PCO_DEBUG` for `path`: the first matching glob's flags, or empty.
fn pco_debug_for(scoped: &[(String, String)], path: &str) -> String {
    scoped.iter().find(|(glob, _)| py::fnmatch(path, glob)).map(|(_, f)| f.clone()).unwrap_or_default()
}

const USAGE: &str = "usage: yakogl-tools glsl2usc [-h] [-o OUTPUT] [--list] [--blend SPEC[:MASK]]
                             [--for GLOB=SPEC[;SPEC...]] [--pco-debug GLOB=FLAGS]
                             [--skip-failures] [--sources LIST] [inputs ...]";

const HELP: &str = "
GLSL-ES -> PowerVR USC blob table (tests/reference/blobs.rs)

positional arguments:
  inputs                GLSL *.vert / *.frag files (required without --sources)

options:
  -h, --help            show this help message and exit
  -o, --output OUTPUT   output Rust file (default: stdout)
  --list                print each shader's name/stage/SHA-256 and exit
  --blend SPEC[:MASK]   also compile every fragment shader for this blend state:
                        srgb,drgb[,sa,da[,eqrgb[,eqa]]] (factor/equation names as
                        in pco_shc PCO_SHC_BLEND) or 'replace', with an optional
                        glColorMask channel set (e.g. src_alpha,inv_src_alpha:rgb,
                        replace:ga) and render target class (@f16x4, @f16, @f32);
                        repeatable
  --for GLOB=SPEC[;SPEC...]
                        blend states for the fragment shaders whose path matches
                        GLOB (fnmatch) INSTEAD of the --blend list; repeatable,
                        first match wins
  --pco-debug GLOB=FLAGS
                        run pco_shc under PCO_DEBUG=FLAGS (Mesa's pco debug
                        options, e.g. no_pred_cf: an `if` under the execution
                        mask with a branch over each arm, not predicated) for
                        every variant of the shaders whose path matches GLOB;
                        repeatable, first match wins. The setting is part of
                        the cache key and is recorded in the table's rows
  --skip-failures       a shader the compiler cannot build is reported and left
                        out of the table (glShaderSource fails for it at runtime)
                        instead of aborting; the skipped names are listed at the end
  --sources LIST        read the inputs, --blend, --for and --skip-failures from
                        LIST (tools/shaders/precompiled.list: its `src`, `blend`,
                        `for` and `skip-failures` lines, paths relative to LIST's
                        directory) before those given here
";

struct Args {
    inputs: Vec<String>,
    output: String,
    list: bool,
    blend: Vec<String>,
    scoped: Vec<String>,
    pco_debug: Vec<String>,
    skip_failures: bool,
    /// `--sources`: the source list the rest is read from first.
    sources: Option<String>,
    /// hidden: print the translated GLSL + reflection instead of compiling
    dump_translation: bool,
}

fn usage_error(msg: &str) -> Error {
    Error::Usage(format!("{USAGE}\nyakogl-tools glsl2usc: error: {msg}"))
}

/// argparse-compatible parsing (long-option prefixes, `--opt=value`,
/// `-oVALUE`, one contiguous run of positional arguments).
fn parse_args(argv: &[String]) -> R<Args> {
    let longs = [
        "--output",
        "--list",
        "--blend",
        "--for",
        "--pco-debug",
        "--skip-failures",
        "--sources",
        "--help",
        "--dump-translation",
    ];
    let mut a = Args {
        inputs: Vec::new(),
        output: "-".into(),
        list: false,
        blend: Vec::new(),
        scoped: Vec::new(),
        pco_debug: Vec::new(),
        skip_failures: false,
        sources: None,
        dump_translation: false,
    };
    let mut runs: Vec<Vec<String>> = Vec::new();
    let mut in_run = false;
    let mut i = 0;
    let mut only_pos = false;
    while i < argv.len() {
        let arg = &argv[i];
        i += 1;
        let is_opt = !only_pos && arg.starts_with('-') && arg != "-";
        if arg == "--" && !only_pos {
            only_pos = true;
            in_run = false;
            continue;
        }
        if !is_opt {
            if !in_run {
                runs.push(Vec::new());
                in_run = true;
            }
            runs.last_mut().unwrap().push(arg.clone());
            continue;
        }
        in_run = false;
        let (opt, inline): (String, Option<String>) = if let Some(rest) = arg.strip_prefix("--") {
            let (name, val) = match rest.find('=') {
                Some(p) => (&rest[..p], Some(rest[p + 1..].to_string())),
                None => (rest, None),
            };
            let full = format!("--{name}");
            let cands: Vec<&str> = longs
                .iter()
                .copied()
                .filter(|l| *l != "--dump-translation" || full == "--dump-translation")
                .filter(|l| l.starts_with(&full))
                .collect();
            let exact = longs.iter().any(|l| *l == full);
            let resolved = if exact {
                full.clone()
            } else if cands.len() == 1 {
                cands[0].to_string()
            } else if cands.is_empty() {
                return Err(usage_error(&format!("unrecognized arguments: {arg}")));
            } else {
                return Err(usage_error(&format!("ambiguous option: {full} could match {}", cands.join(", "))));
            };
            (resolved, val)
        } else {
            let flag = &arg[..2.min(arg.len())];
            let rest = if arg.len() > 2 { Some(arg[2..].to_string()) } else { None };
            match flag {
                "-o" => ("--output".into(), rest),
                "-h" if rest.is_none() => ("--help".into(), None),
                _ => return Err(usage_error(&format!("unrecognized arguments: {arg}"))),
            }
        };
        let takes_value = matches!(opt.as_str(), "--output" | "--blend" | "--for" | "--pco-debug" | "--sources");
        let value = if takes_value {
            match inline {
                Some(v) => Some(v),
                None => {
                    if i < argv.len() && !(argv[i].starts_with('-') && argv[i] != "-") {
                        i += 1;
                        Some(argv[i - 1].clone())
                    } else {
                        let shown = match opt.as_str() {
                            "--output" => "-o/--output",
                            o => o,
                        };
                        return Err(usage_error(&format!("argument {shown}: expected one argument")));
                    }
                }
            }
        } else {
            if inline.is_some() {
                return Err(usage_error(&format!(
                    "argument {opt}: ignored explicit argument {}",
                    py::repr(inline.as_deref().unwrap())
                )));
            }
            None
        };
        match opt.as_str() {
            "--output" => a.output = value.unwrap(),
            "--blend" => a.blend.push(value.unwrap()),
            "--for" => a.scoped.push(value.unwrap()),
            "--pco-debug" => a.pco_debug.push(value.unwrap()),
            "--sources" => a.sources = value,
            "--list" => a.list = true,
            "--skip-failures" => a.skip_failures = true,
            "--dump-translation" => a.dump_translation = true,
            _ => {
                print!("{USAGE}\n{HELP}");
                std::process::exit(0);
            }
        }
    }
    let mut runs = runs.into_iter();
    match runs.next() {
        Some(r) => a.inputs = r,
        None if a.sources.is_some() => {}
        None => return Err(usage_error("the following arguments are required: inputs")),
    }
    let extra: Vec<String> = runs.flatten().collect();
    if !extra.is_empty() {
        return Err(usage_error(&format!("unrecognized arguments: {}", extra.join(" "))));
    }
    Ok(a)
}

/// `--sources LIST`: what the list says, as the command line said it before
/// the list existed -- its `src` paths and `for` globs under LIST's
/// directory, so that names, globs and the rows they make are what
/// tools/glsl2usc.sh's inline arrays made -- ahead of the command line's own.
fn with_sources(mut a: Args) -> R<Args> {
    let Some(path) = a.sources.take() else {
        return Ok(a);
    };
    let l = list::List::read(&path).map_err(|e| Error::Exit(format!("glsl2usc: --sources {e}")))?;
    let inputs: Vec<String> = l.srcs.iter().map(|r| l.abs(r)).collect();
    a.inputs = inputs.into_iter().chain(a.inputs).collect();
    a.blend = l.blends.iter().cloned().chain(a.blend).collect();
    let scoped: Vec<String> = l.rules.iter().map(|(g, specs)| format!("{}={}", l.abs(g), specs.join(";"))).collect();
    a.scoped = scoped.into_iter().chain(a.scoped).collect();
    a.skip_failures |= l.skip_failures;
    Ok(a)
}

pub fn main(argv: &[String]) -> R<()> {
    let args = with_sources(parse_args(argv)?)?;
    /* Before anything is compiled: may this run replace the reference? */
    let ident = tool_identity();
    check_reference(&args.output, &ident)?;
    let mut scoped: Vec<(String, Vec<String>)> = Vec::new();
    for item in &args.scoped {
        let (glob, specs) = py::partition(item, "=");
        if glob.is_empty() || specs.is_empty() {
            exit!("glsl2usc: --for expects GLOB=SPEC[;SPEC...], got '{item}'");
        }
        scoped.push((glob.to_string(), specs.split(';').filter(|x| !x.is_empty()).map(String::from).collect()));
    }
    let debug_scoped = parse_pco_debug(&args.pco_debug)?;
    let debug_for = |path: &str| -> String { pco_debug_for(&debug_scoped, path) };
    let blends_for = |path: &str| -> Vec<String> {
        for (glob, specs) in &scoped {
            if py::fnmatch(path, glob) {
                return specs.clone();
            }
        }
        args.blend.clone()
    };

    if args.dump_translation {
        for path in &args.inputs {
            if path.ends_with(".prebuilt.json") {
                continue;
            }
            let specs = match stage_of(path) {
                Ok("fs") => blends_for(path),
                _ => Vec::new(),
            };
            print!("{}", crate::dump::dump(path, &specs));
        }
        return Ok(());
    }

    let mut shaders: Vec<Shader> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for path in &args.inputs {
        if path.ends_with(".prebuilt.json") {
            continue; // sidecars are picked up via their .frag/.vert
        }
        let mut specs = vec![String::new()];
        if stage_of(path)? == "fs" {
            specs.extend(blends_for(path));
        }
        for spec in &specs {
            let bl = if spec.is_empty() { String::new() } else { format!(" blend={spec}") };
            let (sh, origin) = match resolve_shader(path, spec, Strictness::TableBuild, &debug_for(path)) {
                Ok(Ok(x)) => x,
                Ok(Err(why)) => {
                    eprintln!("glsl2usc: {} skip blend '{}' ({})", py::ljust(py::basename(path), 20), spec, why);
                    continue;
                }
                Err(Error::Exit(msg)) if args.skip_failures => {
                    let short: String = msg.replace('\n', " ").chars().take(300).collect();
                    eprintln!("glsl2usc: {} SKIPPED{}: {}", py::ljust(py::basename(path), 20), bl, short);
                    failed.push(format!("{}{}", py::basename(path), bl));
                    continue;
                }
                Err(e) => return Err(e),
            };
            eprintln!(
                "glsl2usc: {} {}  {} usc_dw={} temps={} sha256={}{}",
                py::ljust(&sh.name, 20),
                sh.stage,
                py::ljust(&format!("[{origin}]"), 18),
                sh.usc.len(),
                sh.temps,
                sh.sha_hex(),
                if spec.is_empty() { String::new() } else { format!("  blend={spec}") }
            );
            shaders.push(sh);
        }
    }
    if !failed.is_empty() {
        eprintln!("glsl2usc: {} shader(s) NOT in the table (compiler failures):", failed.len());
        for f in &failed {
            eprintln!("glsl2usc:   {f}");
        }
    }
    if args.list {
        return Ok(());
    }
    let text = emit::emit(&shaders, &ident)?;
    if !args.output.is_empty() && args.output != "-" {
        py::write_text(&args.output, &text)?;
        eprintln!("glsl2usc: wrote {} ({} shaders)", args.output, shaders.len());
    } else {
        use std::io::Write;
        std::io::stdout().write_all(text.as_bytes()).map_err(|e| Error::Fatal(format!("stdout: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn blend_keys() {
        // src_alpha,inv_src_alpha: 3 | 0x13<<5 | 3<<10 | 0x13<<15
        assert_eq!(blend_key("src_alpha,inv_src_alpha").unwrap(), 0x0009_8e63);
        assert_eq!(blend_key("src_alpha,inv_src_alpha:rgb").unwrap(), 0x2009_8e63);
        assert_eq!(blend_key("replace@f16").unwrap(), abi::SHC_BLEND_KEY_REPLACE | 0x8000_0000);
        assert_eq!(blend_key("replace:ga").unwrap(), abi::SHC_BLEND_KEY_REPLACE | (0x5 << 26));
        match blend_key("replace") {
            Err(Error::Exit(m)) => assert!(m.contains("plain replace")),
            _ => panic!(),
        }
        match blend_key("one,bogus") {
            Err(Error::Exit(m)) => assert_eq!(m, "glsl2usc: unknown blend factor/equation 'bogus'"),
            _ => panic!(),
        }
        match blend_key("one,one@f64") {
            Err(Error::Exit(m)) => {
                assert_eq!(
                    m,
                    "glsl2usc: --blend 'one,one@f64': render target class 'f64' is not one of rgba8, f16x4, f16, f32"
                )
            }
            _ => panic!(),
        }
        match blend_key("one") {
            Err(Error::Exit(m)) => assert_eq!(m, "glsl2usc: --blend needs at least srgb,drgb"),
            _ => panic!(),
        }
    }

    /// `--pco-debug`: parsed, matched first-wins per path, refused malformed,
    /// and part of the cache key only when set (so the shared cache's
    /// existing entries keep their keys).
    #[test]
    fn pco_debug_is_scoped_keyed_and_validated() {
        let a = parse_args(&s(&["--pco-debug", "*/x/*.frag=no_pred_cf", "--pco-d=*.frag=val_skip", "a.frag"])).unwrap();
        let sc = parse_pco_debug(&a.pco_debug).unwrap();
        assert_eq!(pco_debug_for(&sc, "t/x/b.frag"), "no_pred_cf", "first match wins");
        assert_eq!(pco_debug_for(&sc, "t/y/b.frag"), "val_skip");
        assert_eq!(pco_debug_for(&sc, "t/y/b.vert"), "");
        for bad in ["no_pred_cf", "*.frag=", "=no_pred_cf", "*.frag=NO PRED", "*.frag=a,,b"] {
            match parse_pco_debug(&s(&[bad])) {
                Err(Error::Exit(m)) => assert!(m.contains("--pco-debug expects GLOB=FLAGS"), "{bad}: {m}"),
                _ => panic!("{bad} accepted"),
            }
        }
        let plain = cache_key_material("fs", "one,one", "id", "a.frag", false, "");
        assert_eq!(plain, "fs|one,one|id|a.frag|opt0|", "the key without PCO_DEBUG is the one entries already have");
        let dbg = cache_key_material("fs", "one,one", "id", "a.frag", false, "no_pred_cf");
        assert_eq!(dbg, "fs|one,one|id|a.frag|opt0|pco_debug=no_pred_cf|");
    }

    #[test]
    fn variant_environment() {
        let e = variant_env("replace:ga@f16x4").unwrap();
        assert_eq!(e[0].1.as_deref(), Some("f16x4"));
        assert_eq!(e[1].1, None);
        assert_eq!(e[2].1.as_deref(), Some("ga"));
        let e = variant_env("one,one@rgba8").unwrap();
        assert_eq!(e[0].1, None);
        assert_eq!(e[1].1.as_deref(), Some("one,one"));
        assert_eq!(e[2].1.as_deref(), Some("rgba"));
        assert!(variant_env("").unwrap().iter().all(|(_, v)| v.is_none()));
    }

    /// pco_shc's `uses:` line as build-new prints it (Mesa vulkan/pco_shc.c:
    /// `uses: z=%d w=%d discard=%d pntc=%d fbfetch=%d`): `pntc` becomes the
    /// record's `uses_pntc`, the flag gl_test case 83 found missing, and the
    /// other fields read as before.
    #[test]
    fn the_point_coordinate_flag_is_read_from_the_uses_line() {
        for (line, want) in [
            ("uses: z=0 w=1 discard=0 pntc=1 fbfetch=0", (1, 1, 0, 0)),
            ("uses: z=0 w=0 discard=1 pntc=0 fbfetch=1", (0, 0, 1, 1)),
        ] {
            let sh = parse_pco_meta(&format!("temps=4\n{line}\n"), "fs").unwrap();
            assert_eq!((sh.uses_pntc, sh.uses_w, sh.uses_discard, sh.uses_fbfetch), want, "{line}");
        }
    }

    /// `z=` becomes the record's `uses_z`: the fragment shader reads
    /// gl_FragCoord.z, which pco iterates from a Z coefficient set first
    /// (the 32 Qt particle shaders; dropped until 2026-09-24).
    #[test]
    fn the_depth_flag_is_read_from_the_uses_line() {
        for (line, want) in
            [("uses: z=1 w=1 discard=0 pntc=0 fbfetch=0", 1), ("uses: z=0 w=1 discard=0 pntc=1 fbfetch=0", 0)]
        {
            let sh = parse_pco_meta(&format!("temps=4\n{line}\n"), "fs").unwrap();
            assert_eq!(sh.uses_z, want, "{line}");
        }
    }

    #[test]
    fn meta_parsing() {
        let meta = "temps=7\n\
                    desc[set=0 binding=1]: sh.start=0 count=20 stride=20\n\
                    uses: w=1 discard=1 fbfetch=1\n\
                    colormask=0xe\n\
                    blend_consts: sh.start=24 count=4\n\
                    fs_out[x]: po.start=0 count=1\n\
                    push_consts (block): count=4 dwords\n\
                    meta (ISP-feedback control word, driver-supplied): sh.start=24 count=1\n\
                    fs_in[slot=33 VARn]: coeff.start=12 count=8\n\
                    fs_in[slot=32 VARn]: coeff.start=0 count=12\n";
        let sh = parse_pco_meta(meta, "fs").unwrap();
        assert_eq!((sh.temps, sh.uses_w, sh.uses_discard, sh.uses_fbfetch, sh.colormask), (7, 1, 1, 1, 0xe));
        assert_eq!((sh.num_outputs, sh.uniform_dwords, sh.meta_off), (1, 4, 24));
        assert_eq!(sh.blend_consts, Some((24, 4)));
        assert_eq!(sh.front_face_off, None, "this shader does not read gl_FrontFacing");
        assert_eq!(sh.descs[0].repr(), "{'set': 0, 'binding': 1, 'sh_start': 0, 'count': 20, 'stride': 20}");
        // location 1 (2 comps) follows location 0 (3 comps) in the VS output
        assert_eq!((sh.varyings[0].location, sh.varyings[0].components, sh.varyings[0].vtxout_off), (1, 2, 7));
        assert_eq!((sh.varyings[1].location, sh.varyings[1].components, sh.varyings[1].vtxout_off), (0, 3, 4));

        let meta = "vtxouts(vs_output_size)=9\n\
                    vs_out[slot=12 PSIZ]: vtxout.start=8 count=1\n\
                    sysval[SYSTEM_VALUE_VERTEX_ID]: vtxin.start=4 count=1\n\
                    vs_out[slot=32 VARn]: vtxout.start=4 count=4\n\
                    attrib[loc=0 gen=1]: vtxin.start=0 count=4\n";
        let sh = parse_pco_meta(meta, "vs").unwrap();
        assert_eq!(
            (sh.vs_output_size, sh.vs_psize_off, sh.vs_vertex_id_reg, sh.vs_instance_id_reg),
            (9, 8, Some(4), None)
        );
        assert_eq!((sh.attribs[0].location, sh.attribs[0].vi_reg, sh.attribs[0].components), (1, 0, 4));
        assert_eq!(sh.varyings[0].vtxout_off, 4);

        /* gl_FrontFacing: pco_shc allocates the op register after the loaded
         * uniform prefix, so the upload is capped there (upstream 3b827f2) */
        let meta = "temps=6\n\
                    uses: w=0 discard=0 fbfetch=0\n\
                    colormask=0xf\n\
                    fs_out[x]: po.start=0 count=1\n\
                    push_consts (block): count=3 dwords\n\
                    front_face_op: sh.start=3 count=1\n";
        let sh = parse_pco_meta(meta, "fs").unwrap();
        assert_eq!(sh.front_face_off, Some(3));
        assert_eq!(sh.meta_off, SHC_NO_META, "a shader without discard needs no meta word");
    }

    #[test]
    fn argparse_rules() {
        let a = parse_args(&s(&["-oout.rs", "--bl", "one,one", "--for=x=y", "--skip", "a.frag", "b.vert"])).unwrap();
        assert_eq!(a.output, "out.rs");
        assert_eq!(a.blend, ["one,one"]);
        assert_eq!(a.scoped, ["x=y"]);
        assert!(a.skip_failures && !a.list);
        assert_eq!(a.inputs, ["a.frag", "b.vert"]);
        assert!(matches!(parse_args(&s(&["a.frag", "--list", "b.frag"])), Err(Error::Usage(_))));
        assert!(matches!(parse_args(&s(&["--list"])), Err(Error::Usage(_))));
        assert!(matches!(parse_args(&s(&["--blend"])), Err(Error::Usage(_))));
        // the hidden option is not reachable by prefix
        assert!(matches!(parse_args(&s(&["--dump", "a.frag"])), Err(Error::Usage(_))));
    }

    #[test]
    fn meta_of_a_multi_output_shader_with_a_tile_buffer() {
        /* upstream e471a73 / 153810d: a G-buffer shader writes several colour
         * outputs, and the ones that do not fit the pixel's four output
         * registers are stored to tile buffers. */
        let meta = "temps=9\n\
                    uses: w=0 discard=0 fbfetch=0\n\
                    colormask=0xf\n\
                    fs_out[res=4]: po.start=0 count=2\n\
                    fs_out[res=5]: po.start=2 count=2\n\
                    fs_out[res=6]: tilebuf.start=0 count=2\n\
                    push_consts (block): count=8 dwords\n\
                    tile_buffers (driver-supplied addresses): sh.start=28 count=2 stride=2 buffers=1\n";
        let sh = parse_pco_meta(meta, "fs").unwrap();
        assert_eq!(sh.num_outputs, 6, "the registers every output emits from, summed");
        assert_eq!((sh.num_tile_buffers, sh.tile_buffer_sh), (1, 28));
        assert_eq!(sh.uniform_dwords, 8);
    }

    #[test]
    fn a_shader_without_tile_buffers_reports_none() {
        let meta = "temps=5\n\
                    uses: w=0 discard=0 fbfetch=0\n\
                    colormask=0xf\n\
                    fs_out[res=4]: po.start=0 count=1\n\
                    push_consts (block): count=4 dwords\n";
        let sh = parse_pco_meta(meta, "fs").unwrap();
        assert_eq!((sh.num_outputs, sh.num_tile_buffers, sh.tile_buffer_sh), (1, 0, 0));
    }

    #[test]
    fn the_spirv_optimiser_is_off_unless_the_environment_asks_for_it() {
        /* upstream e471a73 added GLSL2USC_OPT; the Python treats unset, empty
         * and "0" alike. The cache key carries it, so the two compilations
         * never share an entry. */
        let cases = [(None, false), (Some(""), false), (Some("0"), false), (Some("1"), true), (Some("yes"), true)];

        for (v, want) in cases {
            // SAFETY: the tool is single-threaded; no other thread reads the
            // environment while this test runs.
            unsafe {
                match v {
                    Some(v) => std::env::set_var("GLSL2USC_OPT", v),
                    None => std::env::remove_var("GLSL2USC_OPT"),
                }
            }
            assert_eq!(opt_pass(), want, "GLSL2USC_OPT={v:?}");
        }
        // SAFETY: as above.
        unsafe { std::env::remove_var("GLSL2USC_OPT") };
    }

    #[test]
    fn a_missing_compiler_is_the_service_failing_not_a_bad_shader() {
        /* Error::Exit reaches `shader-check`'s caller as {"ok": false} --
         * a verdict on the shader. A host with no pco_shc would then call
         * every shader invalid. It has to be Fatal, which comes back as
         * {"error": ...} with no "ok". */
        match not_found("/nonexistent/pco_shc") {
            Error::Fatal(m) => assert!(m.contains("tool not found"), "{m}"),
            other => panic!("a missing compiler must be Fatal, got {other:?}"),
        }
    }

    #[test]
    fn a_shader_with_no_version_and_no_es_keywords_is_still_translated() {
        /* dEQP's uniformTestVertSource (es2fNegativeShaderApiTests.cpp:46):
         * no #version, no attribute/varying/gl_FragColor. The old es_version
         * heuristic called it "not ES", so it was never rewritten and reached
         * glslang as ES text under -V, which fails twice over -- "#version:
         * ES shaders for SPIR-V require version 310 or higher" and
         * "non-opaque uniforms outside a block". That one shape cost about 24
         * mustpass cases in negative_api and state_query. */
        let src = "uniform mediump vec4 vTest;\nuniform mediump mat4 vMatrix;\nvoid main (void)\n{\n\tgl_Position = vMatrix * vTest;\n}\n";
        assert_eq!(glsl::es_version(src).unwrap(), Some(100));
        let (text, _) = glsl::translate(src, "vs", 100).unwrap();
        assert!(text.starts_with("#version 450"), "the rewrite fires: {text}");
        assert!(text.contains("layout(push_constant) uniform"), "the uniforms are in a block: {text}");
        assert!(!text.contains("\nuniform mediump vec4"), "and not left at global scope: {text}");
    }

    #[test]
    fn an_opaque_uniform_stays_outside_the_block() {
        /* The fragment partner mixes a non-opaque ivec4 with a sampler2D.
         * Vulkan GLSL wants the first inside a block and the second outside;
         * putting the sampler in the block would not compile. */
        let src = "uniform mediump ivec4 fTest;\nuniform sampler2D fSampler;\nvoid main (void)\n{\n\tgl_FragColor.xy = vec4(fTest).xy;\n\tgl_FragColor.zw = texture2D(fSampler, vec2(0.0, 0.0)).zw;\n}\n";
        let (text, _) = glsl::translate(src, "fs", 100).unwrap();
        assert!(
            text.contains("layout(set = 0, binding = 0) uniform sampler2D fSampler;"),
            "the sampler is its own declaration with a binding: {text}"
        );
        let block = text.find("layout(push_constant) uniform").expect("a block for the ivec4");
        let close = text[block..].find("};").expect("the block ends") + block;
        assert!(
            !text[block..close].contains("sampler"),
            "and it is not inside the block, which would not compile: {text}"
        );
    }
}

#[cfg(test)]
mod content_id_tests {
    use super::*;

    #[test]
    fn content_id_resolves_a_bare_command_through_path() {
        // `sh` is on every PATH the tests run under; its id must be the hash of
        // the file the shell would run, not "?missing".
        let bare = content_id("sh");
        assert!(!bare.starts_with('?'), "bare command not resolved: {bare}");
        let full = resolve_on_path("sh");
        assert!(full.is_absolute(), "{}", full.display());
        assert_eq!(bare, content_id(full.to_str().unwrap()));
        // A path with a slash is taken as it is.
        assert_eq!(content_id("/nonexistent/dir/sh"), "?missing");
    }
}
