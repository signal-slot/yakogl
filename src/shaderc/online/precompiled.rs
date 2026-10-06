// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The precompiled tier: shaders our own compiler compiled at image-build
//! time, in front of compiling them at run time.
//!
//! The table is what `yakogl-precompile` (tools/precompile) wrote for this
//! build's GPU and ES level from tools/shaders/precompiled.list: every record
//! the compiler's cache held when the list's sources were compiled, its
//! programs linked and their variants asked for, through the same entry
//! points the GL core calls. A row is found by the key the cache uses -- the
//! record key (the source's SHA-256 chained with the zeroed varyings, the
//! attribute map, the constant W, the sampler-state key and a non-default
//! `if` lowering), the stage and the blend key -- so the back end asks, in
//! order: the keep cache, this table, the compiler (`backend`). A hit runs no
//! compile ([`shc_precompiled_hits`] counts them, `shc_online_compiler_runs`
//! does not move); a miss compiles as it would without the table.
//!
//! A row owns nothing: its `ShcShader` points into the image, its `priv`
//! points at the row, and retaining or releasing it does nothing. The back
//! end reads a row's source and facts through `backend::Base`, as it reads a
//! keep record's, so a variant of a row or the link of a row is asked for the
//! same way. Two answers the table carries besides the rows spare the front
//! end at a link and at a program's first draw: the verdict of every link the
//! generator made ([`linked`]) and every row's sampler-state sensitivity
//! ([`sensitivity`]).
//!
//! The rows were compiled with the compiler's switches at their boot values;
//! with any of them changed ([`applies`]) the table is not consulted, so a
//! measurement that flips one compiles as before. The table names the
//! compiler that made it (its toolchain mark, logged once at the first
//! look-up), and a table for another GPU or ES level does not build.

use crate::shaderc::ShcShader;

#[cfg(feature = "precompiled-shaders")]
#[allow(unused_imports)]
use crate::shaderc::{
    pad_attribs, pad_varyings, ShcAttrib, ShcAttribBinding, ShcAttribMap, ShcBlob, ShcBlock, ShcBlockMember, ShcBlocks,
    ShcPrecompiledLink, ShcSource, ShcUniform, ShcVarying, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX,
};
#[cfg(all(feature = "precompiled-shaders", feature = "gpu-vivante"))]
#[allow(unused_imports)]
use crate::shaderc::{pad_io, ShcViv, ShcVivIo, SHC_VIV_MAX_IO};

/* the table: `#[path = "<YAKOGL_PRECOMPILED_TABLE>"] mod table;` (build.rs) */
#[cfg(feature = "precompiled-shaders")]
include!(concat!(env!("OUT_DIR"), "/precompiled_table.rs"));

#[cfg(all(feature = "precompiled-shaders", feature = "gpu-v3d"))]
compile_error!("feature \"precompiled-shaders\": yakogl-precompile makes no table for the V3D");

/// The GPU and ES level this build's compiler makes code for, as a table's
/// toolchain mark names them (`gpu=<..> es2=<0|1>`).
pub const BUILD_GPU: &str = if cfg!(feature = "gpu-vivante") { "gc7000" } else { "powervr" };

/// Does a table made for `gpu` and `es2` fit this build?
pub const fn mark_matches(gpu: &str, es2: bool) -> bool {
    let (a, b) = (gpu.as_bytes(), BUILD_GPU.as_bytes());
    if a.len() != b.len() || es2 != crate::REPORT_ES2 {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/* A table for another GPU or ES level stops the build: its rows are another
 * compiler's code, and nothing at run time could tell. */
#[cfg(feature = "precompiled-shaders")]
const _: () = assert!(
    mark_matches(table::SHC_PRECOMPILED_GPU, table::SHC_PRECOMPILED_ES2),
    "the precompiled table was made for another GPU or ES level (its toolchain mark); regenerate it for this build"
);

/// Look-ups the table answered since boot.
static HITS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// The table is consulted (the C ABI's switch, [`shc_precompiled_set_enabled`]).
static ENABLED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// The mark has been logged.
#[cfg(feature = "precompiled-shaders")]
static MARKED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// `uint32_t shc_precompiled_hits(void)`: how many look-ups the precompiled
/// table answered since boot, wrapping. With `shc_online_compiler_runs` a run
/// says what the table spared: a hit is a compile that did not run.
#[no_mangle]
pub extern "C" fn shc_precompiled_hits() -> u32 {
    HITS.load(core::sync::atomic::Ordering::Relaxed)
}

/// `void shc_precompiled_set_enabled(int on)`: consult the table (1, the
/// default) or compile everything (0), for a measurement against the
/// compiler. Read at each look-up.
#[no_mangle]
pub extern "C" fn shc_precompiled_set_enabled(on: core::ffi::c_int) {
    ENABLED.store(on != 0, core::sync::atomic::Ordering::Relaxed);
}

/// `size_t shc_precompiled_rows(void)`: the table's rows; 0 in a build
/// without one.
#[no_mangle]
pub extern "C" fn shc_precompiled_rows() -> usize {
    rows()
}

/// `const char *shc_precompiled_toolchain(void)`: the table's toolchain mark
/// (`yakogl=<version> commit=<sha>[+dirty] gpu=<gpu> es2=<0|1>`), NUL-
/// terminated; "" in a build without one.
#[no_mangle]
pub extern "C" fn shc_precompiled_toolchain() -> *const core::ffi::c_char {
    #[cfg(feature = "precompiled-shaders")]
    let mark = table::SHC_PRECOMPILED_TOOLCHAIN_C;
    #[cfg(not(feature = "precompiled-shaders"))]
    let mark = "\0";
    mark.as_ptr() as *const core::ffi::c_char
}

/// The table's rows (0 without one).
pub fn rows() -> usize {
    #[cfg(feature = "precompiled-shaders")]
    return table::SHC_PRECOMPILED_TABLE.len();
    #[cfg(not(feature = "precompiled-shaders"))]
    0
}

/// The compiler's switches stand where the rows were compiled: at their boot
/// values. A row is the code of one setting of them, and the record key does
/// not carry them (they are read when a shader compiles), so a table made at
/// the defaults answers only while they hold: the user's (native cube, the
/// projective divide and level, the sampler-state variants, the `if`
/// lowering), the GC7000's measurement knob (`shc_debug_uniidx`), and the
/// switches the host tests flip to compare a compile with and without a
/// step.
fn switches_at_boot() -> bool {
    use crate::shaderc as s;
    let user = s::native_cube_enabled() == (s::PVR_NATIVE_CUBE_DEFAULT != 0)
        && !s::proj_divide_enabled()
        && s::proj_lod_mode() == s::PVR_PROJ_LOD_DEFAULT
        && s::lod_variants_enabled() == (true, true)
        && s::if_lowering() == s::IfLowering::Default;
    #[cfg(feature = "gpu-vivante")]
    let user = user && super::select::uniidx_knobs() == 0 && !super::vivante::alloc::lane_moves_disabled();
    #[cfg(test)]
    let user = user
        && !super::lower::RELOCATE_OFF.load(core::sync::atomic::Ordering::Relaxed)
        && !super::lower::FOLD_OFF.load(core::sync::atomic::Ordering::Relaxed)
        && super::powervr::unroll::early_exit_on();
    user
}

/// The table is consulted now: there is one, it is enabled, and the
/// switches are at their boot values.
pub fn applies() -> bool {
    cfg!(feature = "precompiled-shaders")
        && rows() != 0
        && ENABLED.load(core::sync::atomic::Ordering::Relaxed)
        && switches_at_boot()
}

/// Log the table's toolchain mark, once.
#[cfg(feature = "precompiled-shaders")]
fn mark_once() {
    if !MARKED.swap(true, core::sync::atomic::Ordering::Relaxed) {
        glog!(
            "yakogl: precompiled shaders: {} rows, {} links; toolchain: {}",
            table::SHC_PRECOMPILED_TABLE.len(),
            table::SHC_PRECOMPILED_LINKS.len(),
            table::SHC_PRECOMPILED_TOOLCHAIN
        );
    }
}

/// The row with this identity, as the shader a look-up hands out (its
/// `priv` the row), counted as a hit; `None` when the table has none or
/// is not consulted ([`applies`]).
pub fn find(key: &[u8; 32], stage: u32, blend_key: u32) -> Option<ShcShader> {
    #[cfg(feature = "precompiled-shaders")]
    if applies() {
        mark_once();
        let rows = &table::SHC_PRECOMPILED_TABLE;
        let i = (0..rows.len()).find(|&i| {
            rows[i].stage == stage && rows[i].blend_key == blend_key && &table::SHC_PRECOMPILED_KEYS[i] == key
        })?;
        HITS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let mut sh = rows[i].shader;
        sh.priv_ = &rows[i] as *const ShcBlob as *const core::ffi::c_void;
        return Some(sh);
    }
    let _ = (key, stage, blend_key);
    None
}

/// The row behind a shader [`find`] handed out.
fn row_of(sh: &ShcShader) -> Option<usize> {
    #[cfg(feature = "precompiled-shaders")]
    {
        let rows = &table::SHC_PRECOMPILED_TABLE;
        let size = core::mem::size_of::<ShcBlob>();
        let off = (sh.priv_ as usize).checked_sub(rows.as_ptr() as usize)?;
        if off % size == 0 && off / size < rows.len() {
            return Some(off / size);
        }
    }
    let _ = sh;
    None
}

/// The row behind `sh` as the back end reads a record: its source, facts
/// and blocks. `None` for a shader that is not a row.
pub(super) fn base(sh: &ShcShader) -> Option<super::backend::Base> {
    row_of(sh).and_then(row_base)
}

/// Row `i` as the back end reads a record.
#[cfg(feature = "precompiled-shaders")]
fn row_base(i: usize) -> Option<super::backend::Base> {
    let text = table::SHC_PRECOMPILED_SOURCES[usize::from(table::SHC_PRECOMPILED_SOURCE[i])].text;
    let source = text.as_bytes().strip_suffix(b"\0").unwrap_or(text.as_bytes());
    let row = &table::SHC_PRECOMPILED_TABLE[i];
    Some(super::backend::Base {
        sha: table::SHC_PRECOMPILED_KEYS[i],
        stage: row.stage,
        blend_key: row.blend_key,
        source,
        w_constant: table::SHC_PRECOMPILED_W_CONSTANT[i],
        lod_key: crate::shaderc::LodKey::from_bits(table::SHC_PRECOMPILED_LOD_KEY[i]),
        if_lowering: crate::shaderc::IfLowering::from_u32(table::SHC_PRECOMPILED_IF_LOWERING[i]),
        attrib_map: table::SHC_PRECOMPILED_ATTRIB_MAP[i],
        blocks: table::SHC_PRECOMPILED_BLOCKS[i],
    })
}

/// Row `i` as the back end reads a record: no rows without a table.
#[cfg(not(feature = "precompiled-shaders"))]
fn row_base(_: usize) -> Option<super::backend::Base> {
    None
}

/// The fragment shader's record key a recorded link of `vs` and `fs` (with
/// `fs_blend_key`) left in the program: `fs` itself when it substituted
/// nothing. `None` when the table recorded no such link or is not consulted.
pub fn linked(vs: &[u8; 32], fs: &[u8; 32], fs_blend_key: u32) -> Option<[u8; 32]> {
    #[cfg(feature = "precompiled-shaders")]
    if applies() {
        return table::SHC_PRECOMPILED_LINKS
            .iter()
            .find(|l| &l.vs == vs && &l.fs == fs && l.fs_blend_key == fs_blend_key)
            .map(|l| l.linked_fs);
    }
    let _ = (vs, fs, fs_blend_key);
    None
}

/// The sampler-state sensitivity (`LodKey::bits`) the table recorded for the
/// row behind `sh`: what `lod_sensitivity` answers without the front end.
pub fn sensitivity(sh: &ShcShader) -> Option<u64> {
    let i = row_of(sh)?;
    if !applies() {
        return None;
    }
    #[cfg(feature = "precompiled-shaders")]
    let bits = Some(table::SHC_PRECOMPILED_SENSITIVITY[i]);
    #[cfg(not(feature = "precompiled-shaders"))]
    let bits = {
        let _ = i;
        None
    };
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table names the GPU and ES level it was made for; only this
    /// build's fit (the build-time assertion above is this function).
    #[test]
    fn a_table_for_another_gpu_or_es_level_does_not_fit() {
        assert!(mark_matches(BUILD_GPU, crate::REPORT_ES2));
        assert!(!mark_matches(BUILD_GPU, !crate::REPORT_ES2), "another ES level");
        let other = if BUILD_GPU == "powervr" { "gc7000" } else { "powervr" };
        assert!(!mark_matches(other, crate::REPORT_ES2), "another GPU");
        assert!(!mark_matches("powervrx", crate::REPORT_ES2));
    }

    /// Every scalar and every array a row or a compile puts in a shader (the
    /// pointers by what they point at), as text: two shaders are the same
    /// shader exactly when these are equal.
    #[cfg(feature = "precompiled-shaders")]
    fn sig(s: &ShcShader) -> String {
        use std::fmt::Write as _;
        let mut o = String::new();
        let _ = write!(
            o,
            "{:?}",
            (
                (s.usc_dw, s.temps, s.num_varyings, s.uses_w, s.uses_discard, s.reads_front_facing, s.meta_off),
                (s.fs_front_face_off1, s.num_outputs, s.num_tile_buffers, s.tile_buffer_sh, s.num_attribs),
                (s.vs_output_size, s.vs_psize_off, s.vs_vertex_id_reg1, s.vs_instance_id_reg1, s.uniform_dwords),
                (s.uniform_sh_dwords, s.num_uniforms, s.blend_key, s.uses_fbfetch, s.blend_consts_off, s.uses_pntc),
                (s.uses_z, s.fragcoord_ytf_off1, s.native_cube, s.layout_at_link, s.num_declared),
            )
        );
        let _ = write!(o, " code {:?}", s.usc_words());
        for v in &s.varyings {
            let _ = write!(o, " v{:?}", (v.location, v.components, v.interp, v.vtxout_off, v.name));
        }
        for x in &s.attribs {
            let _ = write!(o, " a{:?}", (x.location, x.components, x.vi_reg, x.format, x.name, x.gl_type));
        }
        for u in s.uniform_table() {
            let _ = write!(o, " u{:?}", (u.name, u.dword_off, u.dwords, u.gl_type, u.array_len, u.binding, u.inner));
        }
        if !s.declared.is_null() {
            // SAFETY: the shader's list of num_declared entries.
            for d in unsafe { core::slice::from_raw_parts(s.declared, usize::from(s.num_declared)) } {
                let _ = write!(o, " d{:?}", (d.name, d.location, d.columns, d.gl_type, d.active));
            }
        }
        #[cfg(feature = "gpu-vivante")]
        {
            let v = &s.viv;
            let _ = write!(
                o,
                " viv{:?}",
                (v.num_in, v.num_out, v.vs_reg_of_in, v.vs_pos_out_reg, v.vs_pointsize_out_reg, v.vs_id_in_reg),
            );
            let _ = write!(
                o,
                "{:?}",
                (
                    v.ps_color_out_reg,
                    v.ps_depth_out_reg,
                    v.vs_load_balancing,
                    v.input_count_unk8,
                    v.const_contents(),
                    v.const_data()
                )
            );
            for x in v.in_.iter().chain(v.out.iter()) {
                let _ = write!(o, " io{:?}", (x.reg, x.slot, x.components, x.interp, x.name));
            }
        }
        o
    }

    /// `compile(stage, text)` through the back end; the shader and the
    /// compiler runs it cost.
    #[cfg(feature = "precompiled-shaders")]
    fn compile(stage: u32, text: &str) -> (ShcShader, u32) {
        let src = std::format!("{text}\0");
        let mut out = ShcShader::ZERO;
        let mut log = [0 as core::ffi::c_char; 256];
        let f = super::super::SHC_ONLINE.compile.unwrap();
        let n0 = super::super::store::shc_online_compiler_runs();
        // SAFETY: a NUL-terminated source, a real output and the log's length.
        let rc = unsafe { f(stage, src.as_ptr() as *const core::ffi::c_char, &mut out, log.as_mut_ptr(), log.len()) };
        assert_eq!(rc, 0, "{}", log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect::<String>());
        (out, super::super::store::shc_online_compiler_runs().wrapping_sub(n0))
    }

    /// The table's own text of row `i`, without its NUL.
    #[cfg(feature = "precompiled-shaders")]
    fn text_of(i: usize) -> &'static str {
        let t = table::SHC_PRECOMPILED_SOURCES[usize::from(table::SHC_PRECOMPILED_SOURCE[i])].text;
        t.strip_suffix('\0').unwrap_or(t)
    }

    /// A source the table holds compiles to its row with no compiler run,
    /// and the row is, field for field, what the compiler makes of it with
    /// the table out of the way; a source the table does not hold compiles.
    #[test]
    #[cfg(feature = "precompiled-shaders")]
    fn a_hit_runs_no_compile_and_is_what_the_compiler_makes() {
        let _serial = super::super::store::test_lock();
        let plain: Vec<usize> = (0..rows())
            .filter(|&i| {
                table::SHC_PRECOMPILED_TABLE[i].blend_key == 0
                    && table::SHC_PRECOMPILED_KEYS[i] == crate::shaderc::sha256::sha256(text_of(i).as_bytes())
            })
            .collect();
        assert!(!plain.is_empty(), "the table has plain rows");
        for &i in &plain {
            let stage = table::SHC_PRECOMPILED_TABLE[i].stage;
            let hits = shc_precompiled_hits();
            let (hit, runs) = compile(stage, text_of(i));
            assert_eq!(runs, 0, "row {i}: a hit runs no compile");
            assert_eq!(shc_precompiled_hits(), hits.wrapping_add(1), "and is counted");
            assert!(core::ptr::eq(hit.priv_, &table::SHC_PRECOMPILED_TABLE[i] as *const ShcBlob as *const _));
            assert!(super::super::backend::is_ours(&hit), "a row is the compiler's");
            shc_precompiled_set_enabled(0);
            let (made, runs) = compile(stage, text_of(i));
            shc_precompiled_set_enabled(1);
            assert_eq!(runs, 1, "with the table off, a compile");
            assert!(sig(&hit) == sig(&made), "row {i}: the row is not what the compiler makes of its source");
            crate::shaderc::release_shader(&made);
            crate::shaderc::release_shader(&hit);
        }
        let miss = "precision mediump float; void main(){ gl_FragColor = vec4(0.3125); } // no row has me\n";
        let hits = shc_precompiled_hits();
        let (sh, runs) = compile(SHC_STAGE_FRAGMENT, miss);
        assert_eq!((runs, shc_precompiled_hits()), (1, hits), "a miss compiles");
        crate::shaderc::release_shader(&sh);
    }

    /// Every program the table recorded, replayed in the GL core's order --
    /// both shaders compiled, linked, the sampler-state question asked of
    /// both, and each of its fragment shader's recorded variants asked and
    /// linked -- runs no compiler at all with the table, and the compiler
    /// every step without it; and each step hands out the same shader (the
    /// same sensitivity) either way. Prints both counts.
    #[test]
    #[cfg(feature = "precompiled-shaders")]
    fn the_tables_programs_run_no_compiler() {
        let _serial = super::super::store::test_lock();
        let b = &super::super::SHC_ONLINE;
        let row = |key: &[u8; 32], blend: u32| {
            (0..rows())
                .find(|&i| &table::SHC_PRECOMPILED_KEYS[i] == key && table::SHC_PRECOMPILED_TABLE[i].blend_key == blend)
        };
        /* the programs: each plain link (blend 0) whose shaders are plain rows */
        let programs: Vec<(usize, usize)> = table::SHC_PRECOMPILED_LINKS
            .iter()
            .filter(|l| l.fs_blend_key == 0)
            .filter_map(|l| Some((row(&l.vs, 0)?, row(&l.fs, 0)?)))
            .filter(|&(v, f)| {
                [v, f]
                    .iter()
                    .all(|&i| table::SHC_PRECOMPILED_KEYS[i] == crate::shaderc::sha256::sha256(text_of(i).as_bytes()))
            })
            .collect();
        assert!(!programs.is_empty(), "the table recorded programs");
        let replay = || -> (u32, Vec<String>) {
            let mut made = Vec::new();
            let n0 = super::super::store::shc_online_compiler_runs();
            for &(v, f) in &programs {
                let (vs, _) = compile(SHC_STAGE_VERTEX, text_of(v));
                let (fs, _) = compile(SHC_STAGE_FRAGMENT, text_of(f));
                crate::shaderc::retain_shader(&fs);
                let mut rfs = fs;
                let mut log = [0 as core::ffi::c_char; 256];
                // SAFETY: two shaders this back end made, and the log's length.
                assert_eq!(unsafe { (b.link.unwrap())(&vs, &mut rfs, log.as_mut_ptr(), log.len()) }, 0);
                for sh in [&vs, &rfs] {
                    // SAFETY: a shader this back end made.
                    let bits = unsafe { (b.lod_sensitivity.unwrap())(sh) };
                    made.push(std::format!("{} sensitivity {bits:#x}", sig(sh)));
                }
                /* the variants the generator asked of this program: its links
                 * of a variant of the same fragment source to the same vertex shader */
                let mut keys: Vec<u32> = table::SHC_PRECOMPILED_LINKS
                    .iter()
                    .filter(|l| l.fs_blend_key != 0 && l.vs == table::SHC_PRECOMPILED_KEYS[v])
                    .filter(|l| row(&l.fs, l.fs_blend_key).is_some_and(|vi| text_of(vi) == text_of(f)))
                    .map(|l| l.fs_blend_key)
                    .collect();
                keys.sort_unstable();
                keys.dedup();
                for k in keys {
                    let mut var = ShcShader::ZERO;
                    // SAFETY: a shader this back end made, a real output, the log's length.
                    let rc = unsafe { (b.blend_variant.unwrap())(&rfs, k, &mut var, log.as_mut_ptr(), log.len()) };
                    assert_eq!(rc, 0);
                    // SAFETY: as the link above.
                    assert_eq!(unsafe { (b.link.unwrap())(&vs, &mut var, log.as_mut_ptr(), log.len()) }, 0);
                    made.push(sig(&var));
                    crate::shaderc::release_shader(&var);
                }
                for sh in [&rfs, &fs, &vs] {
                    crate::shaderc::release_shader(sh);
                }
            }
            (super::super::store::shc_online_compiler_runs().wrapping_sub(n0), made)
        };
        let (with, hits) = replay();
        shc_precompiled_set_enabled(0);
        let (without, compiled) = replay();
        shc_precompiled_set_enabled(1);
        assert_eq!(hits.len(), compiled.len());
        for (i, (h, c)) in hits.iter().zip(&compiled).enumerate() {
            assert!(h == c, "step {i} of the replay: the table's shader is not the compiler's");
        }
        std::println!(
            "precompiled: {} programs, {} shaders: {with} compiler runs with the table, {without} without",
            programs.len(),
            hits.len()
        );
        assert_eq!(with, 0, "the table answers every step of its own programs");
        assert!(without >= 2 * programs.len() as u32, "without it each program compiles");
    }
}
