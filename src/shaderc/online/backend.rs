// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `struct shc_backend` for the in-process compiler: what the GL core calls.
//!
//! The front end ([`super::lexer`] and what follows it) turns GLSL into an
//! [`ir::Module`](super::ir::Module); [`super::codegen`] turns that into a
//! [`ShcShader`]. This file is everything around them — the three entry points
//! of the C ABI, the compiled-shader cache, the two buffers from
//! [`super::store`], and the selection that makes this `g_shaderc`.
//!
//! ```text
//!   glShaderSource + glCompileShader
//!     -> compile(stage, text)
//!          sha256(text) -> the cache          hit: hand back the ShcShader
//!          store::with_scratch                miss: front end -> ir::Module
//!                                                   -> code generator
//!                                                   -> ShcShader in store::keep
//!   glLinkProgram  -> link()  (shaderc::link, the same rule the others use)
//!   a draw with blending -> blend_variant(fs, key): the same source compiled
//!                           again with the blend state lowered into the FS
//! ```
//!
//! **The compiler itself is not finished.** `codegen.rs` reports which
//! milestone is missing; everything here is real, so that the seam is
//! exercised, reviewed and under `tools/check.sh` before the compiler arrives
//! rather than after.
//!
//! ## Selection
//!
//! `g_shaderc` starts as the table, or as this back end in a build without
//! one. [`select_once`] makes this back end the current one on the first
//! `shaderc::backend()` call: the feature is the selection. A `host-test`
//! build never auto-selects: its tests install their own back ends.

use core::ffi::{c_char, c_void};
use core::fmt::Write as _;

use super::codegen::{self, CompileError, Which};
use super::ir;
use super::store;
use crate::shaderc::sha256::sha256;
use crate::shaderc::{LodKey, ShcBackend, ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use crate::util::{cstr_bytes, CStrWriter, Global, EINVAL, ENOMEM, ENOSPC, ENOTSUP};

/// One compiled shader in the `keep` region, owning everything the
/// [`ShcShader`] handed out points at.
///
/// A `ShcShader` the GL core copied keeps raw pointers into this, and the C
/// ABI has nowhere to put a lifetime, so the record counts its holders
/// instead: the GL core takes a reference for every copy it keeps
/// ([`retain`], [`release`] -- the shader object, each program that linked
/// it, each blend variant) and the record's extent goes back to the region
/// when the last one is released. Until 2026-09-23 records were never freed,
/// and after about 200 compiles every compile in the process was refused.
///
/// `Copy` because the arena stores plain data — [`super::arena::Arena`] takes
/// `T: Copy` so that an allocation is never partly initialised.
#[derive(Clone, Copy)]
struct Compiled {
    /// [`MAGIC`] while the record is live; cleared when it is freed, so a
    /// stale copy released twice is caught rather than freeing a stranger.
    magic: u32,
    /// Holders of a copy of `shader`. The cache is not one of them.
    refs: u32,
    /// Everything this record and its shader own in the keep region,
    /// the record included.
    extent: store::KeepExtent,
    sha: [u8; 32],
    stage: u32,
    blend_key: u32,
    /// The source, kept because a blend variant is the same text compiled
    /// again for one blend state.
    source: &'static [u8],
    /// A fragment shader compiled for a program whose vertex shader writes a
    /// constant W ([`link_online`], `shaderc::set_fs_w_constant`): its blend
    /// variants are compiled the same way.
    w_constant: bool,
    /// The sampler-state variant this record was compiled for
    /// ([`online_sampler_variant`]); its blend variants and its link keep
    /// it. [`LodKey::NONE`] for every other compile.
    lod_key: LodKey,
    shader: ShcShader,
    /// The shader's uniform blocks, in this record's keep extent
    /// ([`keep_blocks`]).
    blocks: crate::shaderc::ShcBlocks,
    /// A vertex shader compiled with the link's attribute locations
    /// ([`online_compile_vs_linked`]): the map, in this record's keep
    /// extent. The V3D builds a program's code at link from the sources
    /// ([`link_program`]), and a sampler-state variant of a vertex shader is
    /// compiled again with it ([`online_sampler_variant`]), so the map has
    /// to travel with the source.
    attrib_map: Option<&'static crate::shaderc::ShcAttribMap>,
}

/// How many compiled shaders the cache indexes.
///
/// Overflow is not a failure: the compile still happens and the shader is
/// still right, only the next compile of the same text pays again. The counter
/// below records it, so that "the cache is too small" can be told from "the
/// compiler is slow".
const CACHE_SLOTS: usize = 128;

/// What a live [`Compiled`] carries in `magic`.
const MAGIC: u32 = 0x5348_434b; /* "SHCK" */

/// The cache indexes live records only: a record leaves it when its last
/// holder releases it, so a hit never hands back freed memory, and the next
/// compile of that text compiles again.
struct Cache {
    /// Raw pointers, not references: [`release`] writes the record's count
    /// while the record is indexed here.
    slots: [*const Compiled; CACHE_SLOTS],
    used: usize,
    /// Compiled shaders that did not fit a slot.
    overflowed: u32,
}

static CACHE: Global<Cache> = Global::new(Cache { slots: [core::ptr::null(); CACHE_SLOTS], used: 0, overflowed: 0 });

/// Cache and memory state, for a status line: slots used, compiles that did
/// not fit one, `keep` bytes handed out, and the most scratch any compile has
/// needed. The last two are what sizes `store`'s constants at M4.
pub fn stats() -> (usize, u32, usize, usize) {
    // SAFETY: single-threaded by contract, as everywhere in this crate.
    let c = unsafe { CACHE.get() };
    (c.used, c.overflowed, store::keep_used(), store::scratch_peak())
}

/// The size of one cache record, for a harness accounting for what the keep
/// arena holds.
///
/// `Compiled` stays private; only its size escapes, and it escapes because
/// the alternative was a measurement that could not close. The arena's growth
/// per compile is the source text, this record, the code, the uniform table
/// and the two constant-file arrays; a report that names four of those five
/// and calls the remainder "unaccounted" is how one measurement went looking
/// for reflection names that were not there.
#[cfg(feature = "host-test")]
pub const fn compiled_record_bytes() -> usize {
    core::mem::size_of::<Compiled>()
}

/// Drop every compiled shader and the memory it lives in.
///
/// For a harness measuring one shader at a time. **Every `ShcShader` this
/// module has returned is dangling afterwards**; the cache is cleared in the
/// same call so that a later lookup cannot hand one back.
#[cfg(feature = "host-test")]
pub fn forget_compiled() {
    // SAFETY: single-threaded by contract, as the rest of this cache is.
    let c = unsafe { CACHE.get() };
    c.slots = [core::ptr::null(); CACHE_SLOTS];
    c.used = 0;
    c.overflowed = 0;
    super::store::reset_keep();
}

fn cache_find(sha: &[u8; 32], stage: u32, blend_key: u32) -> Option<&'static Compiled> {
    // SAFETY: single-threaded by contract.
    let c = unsafe { CACHE.get() };
    c.slots[..c.used]
        .iter()
        .filter(|p| !p.is_null())
        // SAFETY: an indexed record is live (release takes it out first).
        .map(|&p| unsafe { &*p })
        .find(|r| r.stage == stage && r.blend_key == blend_key && &r.sha == sha)
}

/// Take a freed record out of the index, so that its slot is free again.
fn cache_remove(rec: *const Compiled) {
    // SAFETY: single-threaded by contract.
    let c = unsafe { CACHE.get() };
    if let Some(i) = c.slots[..c.used].iter().position(|&p| core::ptr::eq(p, rec)) {
        c.used -= 1;
        c.slots[i] = c.slots[c.used];
        c.slots[c.used] = core::ptr::null();
    }
}

fn cache_put(rec: *const Compiled) {
    // SAFETY: single-threaded by contract.
    let c = unsafe { CACHE.get() };
    if c.used == CACHE_SLOTS {
        c.overflowed += 1;
        return;
    }
    c.slots[c.used] = rec;
    c.used += 1;
}

/// Forget every cached shader. Tests only: the records themselves stay
/// where they are, owned by whoever holds them.
#[cfg(test)]
fn cache_clear() {
    // SAFETY: single-threaded by contract.
    let c = unsafe { CACHE.get() };
    c.slots = [core::ptr::null(); CACHE_SLOTS];
    c.used = 0;
    c.overflowed = 0;
}

/// Put a compiled shader, and the source behind it, in the keep arena the
/// compile has open, and close that arena as the record's extent.
///
/// The record starts with one reference, the caller's. `None` when the arena
/// is full (the transaction is dropped, so the compile keeps nothing); the
/// caller turns that into the log line that names the constant to raise.
/* only the V3D's program record and the tests publish without saying */
#[cfg(any(test, feature = "gpu-v3d"))]
#[allow(clippy::too_many_arguments)]
fn publish(
    txn: store::KeepTxn,
    sha: [u8; 32],
    stage: u32,
    blend_key: u32,
    source: &[u8],
    map: Option<&crate::shaderc::ShcAttribMap>,
    shader: ShcShader,
    blocks: crate::shaderc::ShcBlocks,
) -> Result<&'static Compiled, usize> {
    publish_as(txn, sha, stage, blend_key, source, map, Facts::PLAIN, shader, blocks)
}

/// What a compile was asked for beyond its source, stage, blend key, zero
/// list and attribute map: each is a record of its own, and a record
/// compiled again (a blend variant, a link's substitution) keeps them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Facts {
    /// For a program whose vertex shader writes a constant W
    /// (`Compiled::w_constant`).
    w_constant: bool,
    /// A sampler-state variant (`Compiled::lod_key`).
    lod_key: LodKey,
}

impl Facts {
    const PLAIN: Facts = Facts { w_constant: false, lod_key: LodKey::NONE };

    /// The facts `rec` was compiled with.
    fn of(rec: &Compiled) -> Facts {
        Facts { w_constant: rec.w_constant, lod_key: rec.lod_key }
    }
}

/// [`publish`], saying what else the shader was compiled for ([`Facts`]).
#[allow(clippy::too_many_arguments)]
fn publish_as(
    txn: store::KeepTxn,
    sha: [u8; 32],
    stage: u32,
    blend_key: u32,
    source: &[u8],
    map: Option<&crate::shaderc::ShcAttribMap>,
    facts: Facts,
    mut shader: ShcShader,
    blocks: crate::shaderc::ShcBlocks,
) -> Result<&'static Compiled, usize> {
    let keep = store::keep();
    /* The room the compile had, for a refusal: read while `txn` still holds
     * the arena open. Read after it closed, `store::keep()` would open an
     * arena of its own over the largest free run, and name that run's size
     * (and leave that arena open). */
    let full = || keep.left();
    /* What the open arena hands out is good until the extent it becomes is
     * freed, which is after the last holder of the record lets go. */
    let src: &'static [u8] = keep.alloc_copy(source).map_err(|_| full())?;
    let attrib_map: Option<&'static crate::shaderc::ShcAttribMap> = match map {
        Some(m) => Some(&keep.alloc_copy(core::slice::from_ref(m)).map_err(|_| full())?[0]),
        None => None,
    };
    let rec = keep
        .alloc_filled(
            1,
            Compiled {
                magic: MAGIC,
                refs: 1,
                extent: store::KeepExtent::EMPTY,
                sha,
                stage,
                blend_key,
                source: src,
                w_constant: facts.w_constant,
                lod_key: facts.lod_key,
                shader: ShcShader::ZERO,
                blocks,
                attrib_map,
            },
        )
        .map_err(|_| full())?;
    let rec_ptr = rec.as_mut_ptr();
    shader.priv_ = rec_ptr as *const c_void;
    let extent = txn.commit();
    // SAFETY: `rec_ptr` is the one-element allocation just made, initialised
    // by `alloc_filled`, inside the extent just committed; this is the only
    // reference to it.
    let published: &'static mut Compiled = unsafe { &mut *rec_ptr };
    published.shader = shader;
    published.extent = extent;
    let published: &'static Compiled = published;
    cache_put(published);
    Ok(published)
}

/// A compile's block table copied into the keep region, each member's row
/// resolved against the finished shader's uniform table by name (a back end
/// may filter rows, so the reflection's index is not the table's).
fn keep_blocks(
    sh: &ShcShader,
    (blocks, members, rows): codegen::Blocks<'_>,
    keep: &'static crate::shaderc::online::arena::Arena<'static>,
) -> Option<crate::shaderc::ShcBlocks> {
    use crate::shaderc::{ShcBlock, ShcBlockMember, ShcBlocks, SHC_BLOCK_NO_ROW};
    let table = sh.uniform_table();
    /* The table rows declared as arrays, by name as the members below (a
     * back end filters rows; the V3D program's table holds both stages'). */
    let is_array = |u: &crate::shaderc::ShcUniform| {
        let n = crate::util::name_bytes(&u.name);
        rows.iter().flat_map(|r| r.iter()).any(|r| r.is_array && r.name.as_bytes() == n)
    };
    let n_arrays = table.iter().filter(|u| is_array(u)).count();
    let arrays: &'static [u16] = if n_arrays == 0 {
        &[]
    } else {
        let ka = keep.alloc_filled(n_arrays, 0u16).ok()?;
        for (d, k) in ka.iter_mut().zip((0..table.len()).filter(|&k| is_array(&table[k]))) {
            *d = k as u16;
        }
        ka
    };
    if blocks.is_empty() {
        return Some(ShcBlocks { arrays, ..ShcBlocks::EMPTY });
    }
    let kb =
        keep.alloc_filled(blocks.len(), ShcBlock { name: "", data_size: 0, first_member: 0, num_members: 0 }).ok()?;
    for (d, s) in kb.iter_mut().zip(blocks) {
        *d = ShcBlock {
            name: keep.alloc_str(s.name).ok()?,
            data_size: s.data_size,
            first_member: s.first_member,
            num_members: s.num_members,
        };
    }
    let zero = ShcBlockMember {
        name: "",
        row: SHC_BLOCK_NO_ROW,
        offset: 0,
        array_stride: 0,
        matrix_stride: 0,
        row_major: false,
        gl_type: 0,
        array_size: 0,
    };
    let km = keep.alloc_filled(members.len(), zero).ok()?;
    for (d, s) in km.iter_mut().zip(members) {
        let row_name = s.row_name;
        let row = table
            .iter()
            .position(|u| crate::util::name_bytes(&u.name) == row_name.as_bytes())
            .map_or(SHC_BLOCK_NO_ROW, |k| k as u16);
        *d = ShcBlockMember {
            name: keep.alloc_str(s.name).ok()?,
            row,
            offset: s.offset,
            array_stride: s.array_stride,
            matrix_stride: s.matrix_stride,
            row_major: s.row_major,
            gl_type: s.gl_type,
            array_size: s.array_size,
        };
    }
    let kb: &'static [ShcBlock] = kb;
    Some(ShcBlocks { blocks: kb, members: km, arrays })
}

/// The uniform blocks behind a shader: the online back end's record's, or
/// none (a table shader, or no record).
pub fn blocks_of(sh: &ShcShader) -> crate::shaderc::ShcBlocks {
    base_of(sh).map_or(crate::shaderc::ShcBlocks::EMPTY, |c| c.blocks)
}

/// Whether this back end made `sh` (a live record of ours behind it).
pub fn is_ours(sh: &ShcShader) -> bool {
    record_of(sh).is_some()
}

/// The live record behind a shader this back end handed out.
///
/// `None` for a null `priv`, for one another back end set (it does not point
/// into the keep region), and for a record already freed. The GL core calls
/// [`retain`] and [`release`] on every shader it holds whichever back end
/// compiled it, so this is the check that keeps a table shader's `priv` from
/// being read as a record.
fn record_of(sh: &ShcShader) -> Option<*mut Compiled> {
    let p = sh.priv_ as *mut Compiled;
    if !store::in_keep(p as *const u8) || p.align_offset(core::mem::align_of::<Compiled>()) != 0 {
        return None;
    }
    // SAFETY: an aligned pointer into the keep region, which is a static that
    // outlives the process; a stale one reads whatever is there, and the
    // magic says whether that is a live record.
    (unsafe { (*p).magic } == MAGIC).then_some(p)
}

/// The record behind a shader this back end handed out.
fn base_of(sh: &ShcShader) -> Option<&'static Compiled> {
    // SAFETY: a live record is valid until its last holder releases it, and
    // the caller holds `sh`.
    record_of(sh).map(|p| unsafe { &*p })
}

/// The GL core keeps one more copy of `sh`.
///
/// Nothing for a shader this back end did not compile: the table's live in
/// the image.
pub fn retain(sh: &ShcShader) {
    /* test builds: serialised with every compile and release (store::KeepTxn) */
    #[cfg(test)]
    let _serial = store::test_lock();
    if let Some(p) = record_of(sh) {
        // SAFETY: a live record; single-threaded by contract.
        unsafe { (*p).refs += 1 };
    }
}

/// The GL core lets go of one copy of `sh`; the last one gives the shader's
/// memory back to the keep region, and takes it out of the cache.
///
/// Every copy of that shader is dangling afterwards, which is why only the
/// last holder gets here.
pub fn release(sh: &ShcShader) {
    /* test builds: a release frees keep, and a test that holds the lock
     * counts keep; the rig's v3d_shutdown released on a thread that took no
     * lock (store::KeepTxn) */
    #[cfg(test)]
    let _serial = store::test_lock();
    let Some(p) = record_of(sh) else {
        return;
    };
    // SAFETY: a live record; single-threaded by contract.
    let rec = unsafe { &mut *p };
    debug_assert!(rec.refs > 0, "a live record with no holders");
    rec.refs = rec.refs.saturating_sub(1);
    if rec.refs == 0 {
        cache_remove(p);
        let extent = rec.extent;
        rec.magic = 0;
        store::free_keep(extent);
    }
}

/// The bytes of the keep region the record behind `sh` owns: `None` when it
/// is not a live record. Tests only: `gl::program`'s region test, which
/// runs where a whole program compiles (the GC7000 build).
#[cfg(all(test, feature = "gpu-vivante"))]
pub(crate) fn owned_bytes(sh: &ShcShader) -> Option<usize> {
    // SAFETY: a live record.
    record_of(sh).map(|p| unsafe { (*p).extent.bytes() })
}

/// How many holders the record behind `sh` has: `None` when it is not a live
/// record. Tests only.
#[cfg(test)]
pub(crate) fn holders(sh: &ShcShader) -> Option<u32> {
    // SAFETY: a live record.
    record_of(sh).map(|p| unsafe { (*p).refs })
}

fn stage_name(stage: u32) -> &'static str {
    if stage == SHC_STAGE_VERTEX {
        "vertex"
    } else {
        "fragment"
    }
}

/// Write `err` into the compile log and give the errno it maps to.
///
/// The failures are kept apart on purpose. `-ENOTSUP` says the compiler cannot
/// do this yet; `-ENOMEM` says the keep region had no room now and
/// `-ENOSPC` that the program does not fit the scratch arena, each with the
/// constant to raise; and `-EINVAL` says the shader is wrong. An application, and a conformance
/// run, act differently on each; a build that reported all three the same way
/// would make "not implemented" indistinguishable from "your source is
/// invalid".
fn report(log: &mut CStrWriter, stage: u32, err: CompileError) -> i32 {
    match err {
        CompileError::NotImplemented(part) => {
            let _ = write!(
                log,
                "online shaderc: cannot compile this {} shader yet: {} is not implemented",
                stage_name(stage),
                part.describe()
            );
            -ENOTSUP
        }
        /* The keep region holds every compiled shader until it is deleted:
         * full now, it may have room once something is freed (-ENOMEM). The
         * scratch arena is reset for every compile, so a program that does not
         * fit it never will (-ENOSPC). */
        CompileError::OutOfMemory { which: Which::Keep, left } => {
            let _ = write!(
                log,
                "online shaderc: keep full on this {} shader: {} bytes free in its largest run, {} of {} bytes in use; raise {}",
                stage_name(stage),
                left,
                store::keep_in_use(),
                store::keep_size(),
                Which::Keep.knob()
            );
            -ENOMEM
        }
        CompileError::OutOfMemory { which: Which::Scratch, left } => {
            let _ = write!(
                log,
                "online shaderc: this {} shader does not fit the scratch arena ({} bytes left); raise {}",
                stage_name(stage),
                left,
                Which::Scratch.knob()
            );
            -ENOSPC
        }
        CompileError::Reentered => {
            let _ = log.write_str(
                "online shaderc: a compile was started inside a compile; the scratch arena is not reentrant",
            );
            -EINVAL
        }
        /* The front end's own diagnostic, in the shape an application that
         * scrapes `glGetShaderInfoLog` already expects and glslang itself
         * emits: `0:LINE: 'TOKEN' : message`. The token is what makes a log
         * bucketable -- without it every refusal of a kind is the same
         * string, and 1,841 of them were in one dEQP run. */
        CompileError::Rejected { line, tok, msg } => {
            let _ = write!(log, "0:{line}: '{}' : {msg}", tok.as_str());
            -EINVAL
        }
        /* A stage of the back half refused. This is a compiler defect, not a
         * shader defect, so it is -ENOTSUP with the stage and the reason named
         * rather than -EINVAL: telling an application its source is invalid
         * when the code generator could not express it would send the report
         * to the wrong place entirely. The Debug form carries which stage. */
        CompileError::BackEnd(e) => {
            let _ =
                write!(log, "online shaderc: the code generator refused this {} shader: {:?}", stage_name(stage), e);
            -ENOTSUP
        }
        CompileError::BadStage(s) => {
            let _ = write!(
                log,
                "online shaderc: stage {s} is neither vertex ({SHC_STAGE_VERTEX}) nor fragment ({SHC_STAGE_FRAGMENT})"
            );
            -EINVAL
        }
    }
}

/// The keep arena for one compile, at `end`; a refusal is reported in `log`
/// and returned as the errno.
fn open_keep(end: store::KeepEnd, stage: u32, log: &mut CStrWriter) -> Result<store::KeepTxn, i32> {
    match store::begin_keep_at(end) {
        Ok(t) => Ok(t),
        Err(store::KeepRefused::Open) => Err(report(log, stage, CompileError::Reentered)),
        Err(store::KeepRefused::NotLent) => {
            let _ = log.write_str(
                "online shaderc: no keep region: the image has not lent one (shc_online_lend_keep, sized by CONFIG_YAKOGL_SHADERC_KEEP_BYTES)",
            );
            Err(-ENOMEM)
        }
    }
}

/// Compile `glsl` for `stage` and `blend_key`, and cache the result, at
/// `end` of the keep region's largest free run (see [`store::KeepEnd`]).
fn compile_and_publish(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    end: store::KeepEnd,
    log: &mut CStrWriter,
) -> Result<&'static Compiled, i32> {
    compile_and_publish_linked(stage, glsl, blend_key, &ZeroList::EMPTY, None, Facts::PLAIN, end, log)
}

/// [`compile_and_publish`] with the varyings in `zero` read as 0 (see
/// [`link_online`]), or a vertex shader with the attribute locations in
/// `map` ([`online_compile_vs_linked`]). The cache key covers the list and
/// the map, so the substituted shader and the plain one are two records.
/// `facts.w_constant`: a fragment shader for a program whose vertex shader
/// writes a constant W, compiled without exact gradients
/// (`shaderc::fs_w_constant`); `facts.lod_key`: a sampler-state variant
/// (`shaderc::compile_lod_key`). Each a record of its own too.
#[allow(clippy::too_many_arguments)]
fn compile_and_publish_linked(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    zero: &ZeroList,
    map: Option<&crate::shaderc::ShcAttribMap>,
    facts: Facts,
    end: store::KeepEnd,
    log: &mut CStrWriter,
) -> Result<&'static Compiled, i32> {
    let sha = record_key(glsl, zero, map, facts);
    if let Some(hit) = cache_find(&sha, stage, blend_key) {
        /* The caller gets a copy, so it is a holder like any other. */
        retain(&hit.shader);
        return Ok(hit);
    }
    compile_and_publish_keyed(sha, stage, glsl, blend_key, zero, map, facts, end, log)
}

/// The cache key of a record: the source's SHA-256, chained with everything
/// else the compile reads (the zeroed varyings, the attribute map, the
/// constant W, the sampler-state key). The blend key and the stage are the
/// cache's other two keys.
fn record_key(glsl: &[u8], zero: &ZeroList, map: Option<&crate::shaderc::ShcAttribMap>, facts: Facts) -> [u8; 32] {
    let mut sha = sha256(glsl);
    let mut chain = |extra: &[u8]| {
        let mut both = [0u8; 64];
        both[..32].copy_from_slice(&sha);
        both[32..].copy_from_slice(&sha256(extra));
        sha = sha256(&both);
    };
    if zero.n != 0 {
        chain(&zero.key());
    }
    if let Some(map) = map {
        chain(&map_key(map));
    }
    if facts.w_constant {
        chain(b"w-constant");
    }
    if !facts.lod_key.is_none() {
        let mut k = *b"lod-key:        ";
        k[8..].copy_from_slice(&facts.lod_key.bits().to_le_bytes());
        chain(&k);
    }
    sha
}

/// [`compile_and_publish_linked`] past its cache look-up, for the key
/// [`record_key`] gave.
#[allow(clippy::too_many_arguments)]
fn compile_and_publish_keyed(
    sha: [u8; 32],
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    zero: &ZeroList,
    map: Option<&crate::shaderc::ShcAttribMap>,
    facts: Facts,
    end: store::KeepEnd,
    log: &mut CStrWriter,
) -> Result<&'static Compiled, i32> {
    store::note_compile();
    /* The compile's keep arena. Dropped on every failure below, which gives
     * back whatever the code generator had already put in it. */
    let txn = open_keep(end, stage, log)?;
    let mut names = [""; MAX_ZERO];
    let names = zero.names(&mut names);
    /* the switches the code generator reads, put back after the compile */
    struct Restore(bool, LodKey);
    impl Drop for Restore {
        fn drop(&mut self) {
            crate::shaderc::set_fs_w_constant(self.0);
            crate::shaderc::set_compile_lod_key(self.1);
        }
    }
    let _restore = Restore(
        crate::shaderc::set_fs_w_constant(facts.w_constant),
        crate::shaderc::set_compile_lod_key(facts.lod_key),
    );
    let built = store::with_scratch(|scratch| {
        let compiled = match map {
            None => codegen::compile_linked_blocks(stage, glsl, blend_key, names, scratch, store::keep()),
            Some(map) => codegen::compile_vs_mapped_blocks(glsl, map, scratch, store::keep()),
        };
        compiled.and_then(|(sh, b)| {
            /* Copied out of the scratch while it is still lent. */
            let kept = keep_blocks(&sh, b, store::keep())
                .ok_or(CompileError::OutOfMemory { which: Which::Keep, left: store::keep().left() })?;
            Ok((sh, kept))
        })
    });
    let (shader, blocks) = match built {
        Some(Ok(sb)) => sb,
        Some(Err(e)) => return Err(report(log, stage, e)),
        None => return Err(report(log, stage, CompileError::Reentered)),
    };
    match publish_as(txn, sha, stage, blend_key, glsl, map, facts, shader, blocks) {
        Ok(rec) => Ok(rec),
        Err(left) => Err(report(log, stage, CompileError::OutOfMemory { which: Which::Keep, left })),
    }
}

/* ---- the three entry points of `struct shc_backend` ---------------------- */

unsafe extern "C" fn online_compile(
    stage: u32,
    glsl: *const c_char,
    out: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
) -> i32 {
    /* test builds: the cache is read before any keep transaction opens, so
     * the whole entry is serialised (store::KeepTxn) */
    #[cfg(test)]
    let _serial = store::test_lock();
    // SAFETY: the caller passes a null log, or one valid for log_sz bytes.
    let mut log = unsafe { CStrWriter::new(log, log_sz) };
    if glsl.is_null() || out.is_null() {
        let _ = log.write_str("online shaderc: NULL source or output");
        return -EINVAL;
    }
    // SAFETY: glsl is a non-null NUL-terminated string (the GL layer's copy of
    // the application's source).
    let text = unsafe { cstr_bytes(glsl) };
    /* A shader object's record: the top of the region (store::KeepEnd). */
    match compile_and_publish(stage, text, 0, store::KeepEnd::High, &mut log) {
        Ok(rec) => {
            // SAFETY: out is non-null and points at the caller's shc_shader.
            unsafe { out.write(rec.shader) };
            0
        }
        Err(rc) => rc,
    }
}

/// The keep-free count ([`store::keep_frees`]) at which a draw's variant was
/// last refused for want of keep room. While the count has not moved,
/// nothing has been given back and the same compile would be refused again,
/// so a variant compile is refused at once instead of compiled in full on
/// every draw that asks for it.
static VARIANT_NO_ROOM_AT: Global<Option<u64>> = Global::new(None);

/// A draw's variant of `glsl` (at the program's end of the keep region):
/// from the cache, or compiled, or refused at once with
/// [`crate::shaderc::VARIANT_NO_ROOM`] while the keep region is known to be
/// full (see [`VARIANT_NO_ROOM_AT`]).
fn variant_of(
    stage: u32,
    glsl: &[u8],
    blend_key: u32,
    map: Option<&crate::shaderc::ShcAttribMap>,
    facts: Facts,
    log: &mut CStrWriter,
) -> Result<&'static Compiled, i32> {
    let sha = record_key(glsl, &ZeroList::EMPTY, map, facts);
    if let Some(hit) = cache_find(&sha, stage, blend_key) {
        retain(&hit.shader);
        return Ok(hit);
    }
    let frees = store::keep_frees();
    // SAFETY: single-threaded by contract.
    let no_room_at = unsafe { VARIANT_NO_ROOM_AT.get() };
    if *no_room_at == Some(frees) {
        let _ = write!(
            log,
            "online shaderc: keep full: nothing freed since a variant was last refused for room ({} of {} bytes in use); raise {}",
            store::keep_in_use(),
            store::keep_size(),
            Which::Keep.knob()
        );
        return Err(crate::shaderc::VARIANT_NO_ROOM);
    }
    let r =
        compile_and_publish_keyed(sha, stage, glsl, blend_key, &ZeroList::EMPTY, map, facts, store::KeepEnd::Low, log);
    if r.err() == Some(crate::shaderc::VARIANT_NO_ROOM) {
        *no_room_at = Some(frees);
    }
    r
}

/// The variant of a compiled fragment shader for one blend state.
///
/// Blending and colour masks are lowered into the fragment shader on this GPU,
/// so a blend state is a different binary from the same source — which is what
/// the offline table's `BLENDS` list precompiles, one variant per state named
/// ahead of time whether a draw ever asks for it or not. Here there is no
/// list: the source is still in the `keep` arena, so whatever key a draw
/// actually asks for is compiled on demand and cached under that key.
unsafe extern "C" fn online_blend_variant(
    fs: *const ShcShader,
    blend_key: u32,
    out: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
) -> i32 {
    /* test builds: the cache is read before any keep transaction opens, so
     * the whole entry is serialised (store::KeepTxn) */
    #[cfg(test)]
    let _serial = store::test_lock();
    // SAFETY: the caller passes a null log, or one valid for log_sz bytes.
    let mut log = unsafe { CStrWriter::new(log, log_sz) };
    if fs.is_null() || out.is_null() {
        let _ = log.write_str("online shaderc: NULL shader or output");
        return -EINVAL;
    }
    // SAFETY: fs is non-null and points at a shc_shader; read by value because
    // `out` may be the same object.
    let fs = unsafe { fs.read() };
    if blend_key == 0 {
        /* A copy the caller will release like any variant. */
        retain(&fs);
        // SAFETY: out is non-null and points at the caller's shc_shader.
        unsafe { out.write(fs) };
        return 0;
    }
    let Some(base) = base_of(&fs) else {
        let _ = log.write_str("online shaderc: a blend variant of a shader this back end did not compile");
        return -EINVAL;
    };
    if base.stage != SHC_STAGE_FRAGMENT {
        let _ = log.write_str("online shaderc: a blend variant of a vertex shader");
        return -EINVAL;
    }
    match variant_of(SHC_STAGE_FRAGMENT, base.source, blend_key, None, Facts::of(base), &mut log) {
        Ok(rec) => {
            // SAFETY: out is non-null and points at the caller's shc_shader.
            unsafe { out.write(rec.shader) };
            0
        }
        Err(rc) => rc,
    }
}

unsafe extern "C" fn online_link(vs: *const ShcShader, fs: *mut ShcShader, log: *mut c_char, log_sz: usize) -> i32 {
    /* test builds: the cache is read before any keep transaction opens, so
     * the whole entry is serialised (store::KeepTxn) */
    #[cfg(test)]
    let _serial = store::test_lock();
    /* The V3D links a PROGRAM: the table-level rules first (they read the
     * two stages' reflection, which the program record replaces), then the
     * source-level rules and the program compile in `link_online`. */
    #[cfg(feature = "gpu-v3d")]
    {
        if vs.is_null() || fs.is_null() {
            // SAFETY: the caller passes a null log or one valid for log_sz bytes.
            let _ = unsafe { CStrWriter::new(log, log_sz) }.write_str("online shaderc: NULL shader to link");
            return -EINVAL;
        }
        // SAFETY: the caller's contract, passed straight through.
        let rc = unsafe { crate::shaderc::link::link_shaders_v3d(vs, fs, log, log_sz, "online shaderc") };
        if rc != 0 {
            return rc;
        }
        // SAFETY: as below.
        unsafe { link_online(&*vs, &mut *fs, &mut CStrWriter::new(log, log_sz)) }
    }
    #[cfg(not(feature = "gpu-v3d"))]
    if !vs.is_null() && !fs.is_null() {
        // SAFETY: the caller passes a null log or one valid for log_sz bytes;
        // `vs` and `fs` are distinct valid shaders (the struct's contract).
        let rc = unsafe { link_online(&*vs, &mut *fs, &mut CStrWriter::new(log, log_sz)) };
        if rc != 0 {
            return rc;
        }
    }
    // SAFETY: the caller's contract, passed straight through.
    #[cfg(feature = "gpu-powervr")]
    return unsafe { crate::shaderc::link::link_shaders(vs, fs, log, log_sz, "online shaderc") };
    // SAFETY: as above.
    #[cfg(feature = "gpu-vivante")]
    return unsafe { crate::shaderc::link::link_shaders_viv(vs, fs, log, log_sz, "online shaderc") };
}

/* ---- the link rules only the sources can answer ------------------------ */

/// The most varyings one link substitutes: one per fragment input.
const MAX_ZERO: usize = 16;
/// Bytes of one substituted name, as the reflection stores it.
const ZERO_NAME: usize = 32;

/// Fragment varyings to read as 0: owned copies, because the names are found
/// in a scratch arena that is gone before the substituted compile starts.
struct ZeroList {
    names: [[u8; ZERO_NAME]; MAX_ZERO],
    lens: [u8; MAX_ZERO],
    n: usize,
}

impl ZeroList {
    const EMPTY: ZeroList = ZeroList { names: [[0; ZERO_NAME]; MAX_ZERO], lens: [0; MAX_ZERO], n: 0 };

    fn push(&mut self, name: &str) -> bool {
        if self.n == MAX_ZERO || name.len() > ZERO_NAME {
            return false;
        }
        self.names[self.n][..name.len()].copy_from_slice(name.as_bytes());
        self.lens[self.n] = name.len() as u8;
        self.n += 1;
        true
    }

    /// The names as `&str`, in `out`.
    fn names<'s>(&'s self, out: &'s mut [&'s str; MAX_ZERO]) -> &'s [&'s str] {
        for (i, slot) in out.iter_mut().enumerate().take(self.n) {
            /* Copied from a `&str`, so UTF-8. */
            *slot = core::str::from_utf8(&self.names[i][..self.lens[i] as usize]).unwrap_or("");
        }
        &out[..self.n]
    }

    /// The list as bytes for the cache key: each name and a NUL.
    fn key(&self) -> [u8; MAX_ZERO * (ZERO_NAME + 1)] {
        let mut k = [0u8; MAX_ZERO * (ZERO_NAME + 1)];
        let mut at = 0;
        for i in 0..self.n {
            let l = self.lens[i] as usize;
            k[at..at + l].copy_from_slice(&self.names[i][..l]);
            at += l + 1;
        }
        k
    }
}

/// Every fragment input's varying name, whether fed or not.
fn each_input(fs: &ShcShader, mut f: impl FnMut(&[u8])) {
    #[cfg(feature = "gpu-vivante")]
    for i in &fs.viv.in_[..(fs.viv.num_in as usize).min(crate::shaderc::SHC_VIV_MAX_IO)] {
        f(base_name(crate::util::name_bytes(&i.name)));
    }
    #[cfg(not(feature = "gpu-vivante"))]
    for v in &fs.varyings[..(fs.num_varyings as usize).min(fs.varyings.len())] {
        f(base_name(crate::util::name_bytes(&v.name)));
    }
}

/// `m[1]` -> `m`: the varying a per-column input belongs to.
fn base_name(name: &[u8]) -> &[u8] {
    match (name.last(), name.iter().rposition(|&b| b == b'[')) {
        (Some(b']'), Some(i)) => &name[..i],
        _ => name,
    }
}

/// The link rules that need the two SOURCES, which only this back end has
/// (the compiled tables say what a stage writes and reads, not what it
/// declares). Runs before the table-level pairing in `shaderc::link`, on the
/// fragment shader the program holds; 0 or -EINVAL with the log written.
///
/// * **Section 4.3.5.** A varying the fragment shader reads must be DECLARED
///   by the vertex shader, with the same type. Declared and never written is
///   legal: the fragment shader is recompiled with that varying read as 0
///   ([`super::lower::zero_varyings`]) and `*fs` becomes that shader -- the
///   caller's reference to the old one is released and it holds the new.
///   Never declared is the link error, and the log says "not declared".
/// * **Section 4.3.4** (one uniform name space: the same name must have the
///   same type in both stages) **and 4.5.3** (the same uniform must have the
///   same precision). A struct uniform's type is its whole declaration --
///   every member's name, type and resolved precision, used or not -- so a
///   member one stage lacks, or declares `lowp` where the other has
///   `mediump`, is a mismatch even when no stage reads it. For a plain
///   uniform the precision is compared when BOTH stages use it, which is
///   the rule Mesa applies to GLSL ES 1.00: a vertex shader's default
///   `highp` against a fragment shader's `mediump` for an idle uniform is
///   too common in shipping ES 2.0 code to refuse.
///
/// A shader this back end did not compile (no record) is left to the table
/// rules alone.
fn link_online(vs: &ShcShader, fs: &mut ShcShader, log: &mut CStrWriter) -> i32 {
    let (Some(vr), Some(fr)) = (base_of(vs), base_of(fs)) else {
        /* On the V3D the program's code exists only if this back end builds
         * it from the two sources; with no record there is nothing to build
         * from, and a program without code must not link. */
        #[cfg(feature = "gpu-v3d")]
        {
            let _ =
                log.write_str("online shaderc: a shader of this program was not compiled by the in-process compiler");
            return -EINVAL;
        }
        #[cfg(not(feature = "gpu-v3d"))]
        return 0;
    };
    let (vsrc, fsrc) = (vr.source, fr.source);
    let fs_copy = *fs;
    let verdict = store::with_scratch(|a| -> Result<(ZeroList, bool), i32> {
        let text = |t: &[u8]| core::str::from_utf8(t).ok().and_then(|s| a.alloc_str(s).ok());
        let (Some(vsrc), Some(fsrc)) = (text(vsrc), text(fsrc)) else {
            return Ok((ZeroList::EMPTY, false));
        };
        use super::sema::Stage;
        /* Both sources compiled, so neither can fail here short of running
         * out of scratch -- and then the table rules still apply. */
        let (Ok(vi), Ok(fi)) =
            (super::reflect::interface(a, Stage::Vertex, vsrc), super::reflect::interface(a, Stage::Fragment, fsrc))
        else {
            return Ok((ZeroList::EMPTY, false));
        };

        for fu in fi.uniforms {
            let Some(vu) = vi.uniforms.iter().find(|v| v.name == fu.name) else {
                continue;
            };
            if vu.ty != fu.ty {
                let _ = write!(
                    log,
                    "online shaderc: uniform '{}' has another type or precision in each stage (GLSL ES 1.00 \
                     sections 4.3.4, 4.5.3): vertex '{}', fragment '{}'",
                    fu.name, vu.ty, fu.ty
                );
                return Err(-EINVAL);
            }
            if vu.precision != fu.precision && vu.used && fu.used {
                let _ = write!(
                    log,
                    "online shaderc: uniform '{}' has another precision in each stage (GLSL ES 1.00 section \
                     4.3.4): vertex {:?}, fragment {:?}",
                    fu.name, vu.precision, fu.precision
                );
                return Err(-EINVAL);
            }
        }

        let mut zero = ZeroList::EMPTY;
        let mut bad: Option<(&str, bool)> = None;
        each_input(&fs_copy, |name| {
            if bad.is_some() {
                return;
            }
            let Some(fv) = fi.reflection.varyings.iter().find(|v| v.name.as_bytes() == name) else {
                return;
            };
            match vi.reflection.varyings.iter().find(|v| v.name == fv.name) {
                None => bad = Some((fv.name, true)),
                Some(vv) if vv.gl_type != fv.gl_type || vv.count != fv.count => bad = Some((fv.name, false)),
                Some(_) => {}
            }
        });
        if let Some((name, undeclared)) = bad {
            let _ = write!(
                log,
                "online shaderc: fragment varying '{}' is {}; GLSL ES 1.00 section 4.3.5: a varying the \
                 fragment shader uses must be declared by the vertex shader, with the same type",
                name,
                if undeclared {
                    "not declared in the vertex shader"
                } else {
                    "declared with another type in the vertex shader"
                }
            );
            return Err(-EINVAL);
        }
        /* The varying budget (GLSL ES 1.00 Appendix A 7), against the
         * MAX_VARYING_VECTORS the GL reports. Only the sources know the
         * varyings' types: the compiled tables list components per
         * location, and a mat2 and a vec2[2] read the same there. */
        if let Err(unfit) = super::reflect::varying_budget(&vi, &fi, crate::gl::context::MAX_VARYING_VECTORS) {
            let _ = write!(log, "online shaderc: link error: {unfit}");
            return Err(-EINVAL);
        }

        /* Written means STATICALLY written: some store to one of its
         * locations anywhere in the vertex IR, taken or not. Decided from
         * the IR and not from the compiled tables, because the tables differ
         * by GPU -- the PowerVR lists a declared, unwritten varying as an
         * output (its register is never written, so the fragment shader
         * would read whatever was there), the GC7000 omits it. */
        let Ok(vl) = super::lower::compile_to_ir(a, Stage::Vertex, vsrc) else {
            return Ok((ZeroList::EMPTY, false));
        };
        /* A constant W (the vertex shader's gl_Position.w, strictly: see
         * ir::Module::constant_position_w) makes every varying affine: the
         * fragment shader is then compiled without exact gradients, when it
         * has any (PowerVR, shaderc::pvr_proj_lod). Asked only then, so a
         * program pays the fragment front end again only when it gains. */
        #[cfg(feature = "gpu-powervr")]
        let affine = vl.module.constant_position_w().is_some()
            && super::lower::compile_to_ir(a, Stage::Fragment, fsrc)
                .is_ok_and(|fl| super::powervr::select::samples_with_plane_gradients(&fl.module));
        #[cfg(not(feature = "gpu-powervr"))]
        let affine = false;
        let written = |name: &str| {
            vl.module
                .outputs
                .iter()
                .filter(|o| o.kind == ir::IoKind::Varying && base_name(o.name.as_bytes()) == name.as_bytes())
                .any(|o| {
                    vl.module.instrs.iter().any(
                        |op| matches!(op, ir::Op::StoreOutput { slot: ir::IoSlot::Varying(l), .. } if *l == o.location),
                    )
                })
        };
        let mut full = false;
        each_input(&fs_copy, |name| {
            let Some(fv) = fi.reflection.varyings.iter().find(|v| v.name.as_bytes() == name) else {
                return;
            };
            /* Declared (checked above) and never written: read as 0. */
            if !written(fv.name) {
                let mut have = [""; MAX_ZERO];
                if !zero.names(&mut have).contains(&fv.name) && !zero.push(fv.name) {
                    full = true;
                }
            }
        });
        if full {
            /* More than the table can hold: leave it to the table rules,
             * which refuse it by name rather than half-substitute. */
            return Ok((ZeroList::EMPTY, affine));
        }
        Ok((zero, affine))
    });
    let (zero, affine) = match verdict {
        Some(Ok(z)) => z,
        Some(Err(rc)) => return rc,
        /* A compile inside a compile. Elsewhere the table rules still apply
         * and the link goes on; on the V3D the program cannot be built, so
         * the link fails with the reason. */
        #[cfg(feature = "gpu-v3d")]
        None => return report(log, SHC_STAGE_FRAGMENT, CompileError::Reentered),
        #[cfg(not(feature = "gpu-v3d"))]
        None => return 0,
    };
    #[cfg(feature = "gpu-v3d")]
    {
        let _ = affine;
        link_program(vr, fr, &zero, fs, log)
    }
    #[cfg(not(feature = "gpu-v3d"))]
    {
        let rc = link_substituted(fr, &zero, affine, fs, log);
        /* The GC7000's two stages share one constant file, and the literals
         * a stage folds into it count as well as its uniforms: a program
         * whose two files do not fit would draw with registers missing. */
        #[cfg(feature = "gpu-vivante")]
        if rc == 0 {
            if let Err((v, f)) = crate::vivante::shader::constant_files_fit(vs, fs) {
                let _ = write!(
                    log,
                    "online shaderc: link error: the constant registers the two stages need (vertex {v}, \
                     fragment {f}: uniforms and literals) exceed the {} this GPU has",
                    crate::vivante::regs::GC_NUM_CONSTANTS
                );
                return -EINVAL;
            }
        }
        rc
    }
}

/// The fragment shader recompiled with the varyings in `zero` read as 0,
/// and without exact gradients when `w_constant`, in place of `*fs` (see
/// [`link_online`]). Not on the V3D, where the program compile applies the
/// list.
#[cfg(not(feature = "gpu-v3d"))]
fn link_substituted(fr: &Compiled, zero: &ZeroList, w_constant: bool, fs: &mut ShcShader, log: &mut CStrWriter) -> i32 {
    /* a sampler-state variant stays one (`Compiled::lod_key`) */
    let facts = Facts { w_constant, lod_key: fr.lod_key };
    if zero.n == 0 && !w_constant {
        return 0;
    }
    match compile_and_publish_linked(
        SHC_STAGE_FRAGMENT,
        fr.source,
        fr.blend_key,
        zero,
        None,
        facts,
        store::KeepEnd::Low,
        log,
    ) {
        Ok(rec) => {
            let old = *fs;
            *fs = rec.shader;
            release(&old);
            0
        }
        Err(rc) => rc,
    }
}

/// The cache key and `Compiled::blend_key` of a V3D program record: never a
/// blend key (the V3D has no blend variants, `codegen::compile_linked` refuses
/// every non-zero one), so a program record can never be taken for a stage.
#[cfg(feature = "gpu-v3d")]
const V3D_PROGRAM_KEY: u32 = u32::MAX;

/// Build (or find) the V3D program of `vr` + `fr` and make it the program's
/// fragment shader (docs/online-shaderc/v3d-backend.md, "M6").
///
/// The record is a `Compiled` like any other: one extent of keep, counted
/// holders, freed when the last holder lets go. The caller's copy of the
/// fragment stage is released and replaced by a reference to the program,
/// exactly as the zero-varying recompile above replaces it.
///
/// The key: the two sources' hashes (the vertex one covers the link's attribute
/// map when it has one), the zero list, and the two inputs of the
/// compile that are not in the sources (the primitive class and the VPM
/// size). The compiler's own version is not in it: the cache lives in RAM and
/// starts empty in every process, so every record in it was built by the
/// compiler of this binary.
#[cfg(feature = "gpu-v3d")]
fn link_program(vr: &Compiled, fr: &Compiled, zero: &ZeroList, fs: &mut ShcShader, log: &mut CStrWriter) -> i32 {
    let mut key = [0u8; 32 * 3 + 8 + 8];
    key[..32].copy_from_slice(&vr.sha);
    key[32..64].copy_from_slice(&fr.sha);
    key[64..96].copy_from_slice(&sha256(&zero.key()));
    key[96..100].copy_from_slice(&u32::from(crate::shaderc::SHC_V3D_PRIM_TRIANGLES).to_le_bytes());
    key[100..104].copy_from_slice(&codegen::V3D_VPM_BYTES.to_le_bytes());
    /* the fragment record's sampler-state key (online_sampler_variant):
     * the program compiled with it (`fr.sha` covers it too; kept here so the
     * record's own key says it) */
    key[104..112].copy_from_slice(&fr.lod_key.bits().to_le_bytes());
    /* `vr.sha` of a vertex shader compiled with the link's attribute
     * locations covers the map too (compile_and_publish_linked chains
     * map_key into it), so two layouts of one source are two programs. */
    let sha = sha256(&key);

    let rec = if let Some(hit) = cache_find(&sha, SHC_STAGE_FRAGMENT, V3D_PROGRAM_KEY) {
        retain(&hit.shader);
        hit
    } else {
        store::note_compile();
        /* The program's record: the bottom of the region (store::KeepEnd). */
        let txn = match open_keep(store::KeepEnd::Low, SHC_STAGE_FRAGMENT, log) {
            Ok(t) => t,
            Err(rc) => return rc,
        };
        let mut names = [""; MAX_ZERO];
        let names = zero.names(&mut names);
        struct Restore(LodKey);
        impl Drop for Restore {
            fn drop(&mut self) {
                crate::shaderc::set_compile_lod_key(self.0);
            }
        }
        let _restore = Restore(crate::shaderc::set_compile_lod_key(fr.lod_key));
        let built = store::with_scratch(|a| {
            codegen::link_sources(vr.source, vr.attrib_map, fr.source, names, a, store::keep()).and_then(|(sh, b)| {
                /* Copied out of the scratch while it is still lent, as in
                 * `compile_and_publish_linked`. */
                let kept = keep_blocks(&sh, b, store::keep())
                    .ok_or(CompileError::OutOfMemory { which: Which::Keep, left: store::keep().left() })?;
                Ok((sh, kept))
            })
        });
        let (shader, blocks) = match built {
            Some(Ok(sb)) => sb,
            /* The V3D compiles the program, not a stage: a refusal here may
             * be the vertex shader's as well as the fragment shader's, and
             * `report` would name the fragment stage (the program takes its
             * place). Say "program"; the error's Debug form names the part
             * that refused when it carries one. */
            Some(Err(CompileError::BackEnd(e))) => {
                let _ =
                    write!(log, "online shaderc: the code generator refused this program (vertex + fragment): {:?}", e);
                return -ENOTSUP;
            }
            Some(Err(e)) => return report(log, SHC_STAGE_FRAGMENT, e),
            None => return report(log, SHC_STAGE_FRAGMENT, CompileError::Reentered),
        };
        match publish(txn, sha, SHC_STAGE_FRAGMENT, V3D_PROGRAM_KEY, fr.source, None, shader, blocks) {
            Ok(rec) => rec,
            Err(left) => {
                return report(log, SHC_STAGE_FRAGMENT, CompileError::OutOfMemory { which: Which::Keep, left });
            }
        }
    };
    let old = *fs;
    *fs = rec.shader;
    release(&old);
    0
}

/// The bytes of a link's attribute map that decide the compile: each
/// binding's name, location and column count, in order.
fn map_key(map: &crate::shaderc::ShcAttribMap) -> [u8; crate::shaderc::SHC_MAX_DECLARED_ATTRIBS * 34 + 1] {
    let mut k = [0u8; crate::shaderc::SHC_MAX_DECLARED_ATTRIBS * 34 + 1];
    k[0] = map.count;
    for (i, b) in map.b[..(map.count as usize).min(map.b.len())].iter().enumerate() {
        let at = 1 + 34 * i;
        for (d, c) in k[at..at + 32].iter_mut().zip(b.name.iter()) {
            *d = *c as u8;
        }
        k[at + 32] = b.location;
        k[at + 33] = b.columns;
    }
    k
}

/// `compile_vs_linked`: the vertex shader again, with the attribute
/// locations glLinkProgram decided (SHADERC_NOTES.md, "Attribute registers
/// decided at link"). A program's copy, so at the program's end of the keep
/// region, and cached under the source and the map.
unsafe extern "C" fn online_compile_vs_linked(
    glsl: *const c_char,
    map: *const crate::shaderc::ShcAttribMap,
    out: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
) -> i32 {
    #[cfg(test)]
    let _serial = store::test_lock();
    // SAFETY: the caller passes a null log, or one valid for log_sz bytes.
    let mut log = unsafe { CStrWriter::new(log, log_sz) };
    if glsl.is_null() || map.is_null() || out.is_null() {
        let _ = log.write_str("online shaderc: NULL source, map or output");
        return -EINVAL;
    }
    // SAFETY: a NUL-terminated source (the shader object's copy) and a map
    // the caller holds across the call.
    let (text, map) = unsafe { (cstr_bytes(glsl), &*map) };
    match compile_and_publish_linked(
        SHC_STAGE_VERTEX,
        text,
        0,
        &ZeroList::EMPTY,
        Some(map),
        Facts::PLAIN,
        store::KeepEnd::Low,
        &mut log,
    ) {
        Ok(rec) => {
            // SAFETY: out is non-null and points at the caller's shc_shader.
            unsafe { out.write(rec.shader) };
            0
        }
        Err(rc) => rc,
    }
}

/* ---- sampler-state variants (shaderc::LodKey) ----------------------------- */

/// Does this build's compiler honour [`LodKey::sample_kill`]? The PowerVR's
/// does (`powervr::select`); the GC7000 and the V3D take the sample state as
/// pipeline state and never ask for the variant.
const SAMPLE_KILL_VARIANTS: bool = !cfg!(not(feature = "gpu-powervr"));

/// The [`LodKey`] sensitivity of a compiled record: its source through the
/// front end again, and the look-ups whose code the key would change --
/// explicit levels that can fall in (0, 0.5] (`lodkey::mag_switch_
/// sensitivity`, switch (b), in a build that reports ES 2.0 only), and on the PowerVR the fragment look-ups that
/// take exact gradients (`powervr::select::gradients_for`, switch (a)).
/// On the PowerVR a fragment shader with no `discard` of its own is also
/// sensitive to the draw's sample state ([`LodKey::sample_kill`]), whatever
/// the two switches say.
/// On the V3D a program record ([`link_program`]) is sensitive to the
/// format of every fragment binding below 16 a look-up reads
/// ([`LodKey::ret32`]: an R32F texture needs the 32-bit TMU output); nothing
/// else on the V3D is (a vertex look-up of a depth texture is refused at the
/// draw, `v3d::texture::refusal_for`). [`LodKey::NONE`] for a source the
/// front end cannot run again here.
fn sensitivity_of(rec: &Compiled) -> LodKey {
    #[cfg(feature = "gpu-v3d")]
    if rec.blend_key == V3D_PROGRAM_KEY && rec.stage == SHC_STAGE_FRAGMENT {
        return v3d_ret32_sensitivity(rec.source);
    }
    let (mip, switch) = crate::shaderc::lod_variants_enabled();
    let kill = SAMPLE_KILL_VARIANTS && rec.stage == SHC_STAGE_FRAGMENT;
    if cfg!(feature = "gpu-v3d") || !(mip || switch || kill) {
        return LodKey::NONE;
    }
    /* the gradients the record's own compile asked about: its constant W,
     * and no key */
    struct Restore(bool, LodKey);
    impl Drop for Restore {
        fn drop(&mut self) {
            crate::shaderc::set_fs_w_constant(self.0);
            crate::shaderc::set_compile_lod_key(self.1);
        }
    }
    let _restore =
        Restore(crate::shaderc::set_fs_w_constant(rec.w_constant), crate::shaderc::set_compile_lod_key(LodKey::NONE));
    let stage = match rec.stage {
        SHC_STAGE_VERTEX => super::sema::Stage::Vertex,
        SHC_STAGE_FRAGMENT => super::sema::Stage::Fragment,
        _ => return LodKey::NONE,
    };
    store::with_scratch(|a| {
        let Some(text) = core::str::from_utf8(rec.source).ok().and_then(|s| a.alloc_str(s).ok()) else {
            return LodKey::NONE;
        };
        let Ok(l) = super::lower::compile_to_ir(a, stage, text) else {
            return LodKey::NONE;
        };
        let m = &l.module;
        /* only a context that reports ES 2.0: dEQP judges an ES 3-capable
         * context against its seamless reference, whose switch is at 0 (the
         * hardware's own), and the remap would fail those cases */
        let mag_switch = if switch && crate::REPORT_ES2 { super::lodkey::mag_switch_sensitivity(m) } else { 0 };
        #[cfg(feature = "gpu-powervr")]
        let mip_free = if mip && rec.stage == SHC_STAGE_FRAGMENT {
            m.instrs.iter().fold(0u32, |acc, op| match *op {
                ir::Op::Tex { sampler, .. }
                    if sampler < 32 && super::powervr::select::gradients_for(m, op).is_some() =>
                {
                    acc | 1 << sampler
                }
                _ => acc,
            })
        } else {
            0
        };
        #[cfg(not(feature = "gpu-powervr"))]
        let mip_free = 0;
        let sample_kill = kill && !m.instrs.iter().any(|op| matches!(op, ir::Op::Discard));
        LodKey { mip_free, mag_switch, sample_kill, ret32: 0 }
    })
    .unwrap_or(LodKey::NONE)
}

/// The fragment bindings below 16 that `fs` reads with a look-up: the
/// program record's [`LodKey::ret32`] sensitivity.
#[cfg(feature = "gpu-v3d")]
fn v3d_ret32_sensitivity(fs: &[u8]) -> LodKey {
    struct Restore(LodKey);
    impl Drop for Restore {
        fn drop(&mut self) {
            crate::shaderc::set_compile_lod_key(self.0);
        }
    }
    let _restore = Restore(crate::shaderc::set_compile_lod_key(LodKey::NONE));
    store::with_scratch(|a| {
        let Some(text) = core::str::from_utf8(fs).ok().and_then(|s| a.alloc_str(s).ok()) else {
            return LodKey::NONE;
        };
        let Ok(l) = super::lower::compile_to_ir(a, super::sema::Stage::Fragment, text) else {
            return LodKey::NONE;
        };
        let ret32 = l.module.instrs.iter().fold(0u32, |acc, op| match *op {
            ir::Op::Tex { sampler, .. } if sampler < 16 => acc | 1 << sampler,
            _ => acc,
        });
        LodKey { ret32, ..LodKey::NONE }
    })
    .unwrap_or(LodKey::NONE)
}

/// `lod_sensitivity`: which bindings' filter state `sh`'s code depends on
/// ([`LodKey::bits`]); 0 for a shader this back end did not compile.
unsafe extern "C" fn online_lod_sensitivity(sh: *const ShcShader) -> u64 {
    #[cfg(test)]
    let _serial = store::test_lock();
    if sh.is_null() {
        return 0;
    }
    // SAFETY: sh is non-null and points at a shc_shader.
    let sh = unsafe { sh.read() };
    base_of(&sh).map_or(0, |rec| sensitivity_of(rec).bits())
}

/// The V3D programs' draw-time sampler variants asked for, built and
/// refused, since boot: each one is a fragment compile (or a cache hit) and
/// a whole-program link at draw time. The GL core keeps a program's
/// variants (`lod_variant_for`), so a steady scene stops asking; a count
/// that grows every frame is a program re-linking every frame. The V3D
/// back end prints it with its arena high-water lines.
#[cfg(feature = "gpu-v3d")]
static V3D_VARIANTS_BUILT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
#[cfg(feature = "gpu-v3d")]
static V3D_VARIANTS_REFUSED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// `(built, refused)`: the V3D programs' draw-time sampler variants since
/// boot ([`V3D_VARIANTS_BUILT`]).
#[cfg(feature = "gpu-v3d")]
pub fn v3d_sampler_variants() -> (u32, u32) {
    use core::sync::atomic::Ordering::Relaxed;
    (V3D_VARIANTS_BUILT.load(Relaxed), V3D_VARIANTS_REFUSED.load(Relaxed))
}

/// Tests of the V3D back end's print: the counts moved as a draw-time
/// variant would move them (the counting itself is
/// `a_v3d_program_has_a_ret32_variant`'s). Under `store::test_lock`.
#[cfg(all(test, feature = "gpu-v3d"))]
pub(crate) fn v3d_sampler_variants_add(built: u32, refused: u32) {
    use core::sync::atomic::Ordering::Relaxed;
    V3D_VARIANTS_BUILT.fetch_add(built, Relaxed);
    V3D_VARIANTS_REFUSED.fetch_add(refused, Relaxed);
}

/// `sampler_variant`: `sh` compiled again for `key` ([`LodKey::bits`]),
/// with the attribute map and the constant W it was compiled with. A draw's
/// variant, so at the program's end of the keep region; cached under the
/// source and the key like a blend variant. The caller links a fragment
/// result (`link`), which keeps the key.
unsafe extern "C" fn online_sampler_variant(
    sh: *const ShcShader,
    key: u64,
    out: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
) -> i32 {
    #[cfg(test)]
    let _serial = store::test_lock();
    // SAFETY: the caller passes a null log, or one valid for log_sz bytes.
    let mut log = unsafe { CStrWriter::new(log, log_sz) };
    if sh.is_null() || out.is_null() {
        let _ = log.write_str("online shaderc: NULL shader or output");
        return -EINVAL;
    }
    // SAFETY: sh is non-null and points at a shc_shader; read by value
    // because `out` may be the same object.
    let sh = unsafe { sh.read() };
    if key == 0 {
        retain(&sh);
        // SAFETY: out is non-null and points at the caller's shc_shader.
        unsafe { out.write(sh) };
        return 0;
    }
    let Some(base) = base_of(&sh) else {
        let _ = log.write_str("online shaderc: a sampler-state variant of a shader this back end did not compile");
        return -EINVAL;
    };
    /* The V3D: the program record's variant is its fragment stage compiled
     * with the key, which the caller links to the vertex shader as any
     * fragment variant (lod_variant_for); the link builds the program with
     * the stage's key (`link_program`). Only `ret32` is a V3D key. */
    #[cfg(feature = "gpu-v3d")]
    if base.blend_key == V3D_PROGRAM_KEY {
        use core::sync::atomic::Ordering::Relaxed;
        let k = LodKey::from_bits(key);
        if k.mip_free != 0 || k.mag_switch != 0 || k.sample_kill {
            V3D_VARIANTS_REFUSED.fetch_add(1, Relaxed);
            let _ = write!(log, "online shaderc: the V3D has no sampler-state variant for key {key:#x}");
            return -EINVAL;
        }
        let facts = Facts { w_constant: false, lod_key: k };
        return match variant_of(SHC_STAGE_FRAGMENT, base.source, 0, None, facts, &mut log) {
            Ok(rec) => {
                V3D_VARIANTS_BUILT.fetch_add(1, Relaxed);
                // SAFETY: out is non-null and points at the caller's shc_shader.
                unsafe { out.write(rec.shader) };
                0
            }
            Err(rc) => {
                V3D_VARIANTS_REFUSED.fetch_add(1, Relaxed);
                rc
            }
        };
    }
    if base.blend_key != 0 || !(base.stage == SHC_STAGE_VERTEX || base.stage == SHC_STAGE_FRAGMENT) {
        let _ = log.write_str("online shaderc: a sampler-state variant of a blend variant");
        return -EINVAL; /* a variant of the plain shader only; blending comes after */
    }
    let facts = Facts { w_constant: base.w_constant, lod_key: LodKey::from_bits(key) };
    match variant_of(base.stage, base.source, 0, base.attrib_map, facts, &mut log) {
        Ok(rec) => {
            // SAFETY: out is non-null and points at the caller's shc_shader.
            unsafe { out.write(rec.shader) };
            0
        }
        Err(rc) => rc,
    }
}

/* ---- registration -------------------------------------------------------- */

/// The in-process back end.
pub static SHC_ONLINE: ShcBackend = ShcBackend {
    compile: Some(online_compile),
    link: Some(online_link),
    blend_variant: Some(online_blend_variant),
    compile_vs_linked: Some(online_compile_vs_linked),
    lod_sensitivity: Some(online_lod_sensitivity),
    sampler_variant: Some(online_sampler_variant),
};

/// `shc_online_register()`: make this the current back end.
///
/// # Safety
/// Single-threaded by contract, like the other back ends' registration hooks.
#[no_mangle]
pub unsafe extern "C" fn shc_online_register() -> *const ShcBackend {
    selection_made();
    // SAFETY: a plain store; no reference to the static is formed.
    unsafe { crate::shaderc::g_shaderc = &SHC_ONLINE };
    &SHC_ONLINE
}

/// Set once a back end is in place: by the first `shaderc::backend()` call
/// ([`select_once`]), or by an explicit registration before it.
static SELECTED: Global<bool> = Global::new(false);

/// An explicit registration (`shc_online_register`, `shc_offline_register`)
/// is the selection: the first `shaderc::backend()` call keeps it instead of
/// installing this back end over it (gles_shaderc.h: "select a backend
/// explicitly, before the first GL call").
pub fn selection_made() {
    // SAFETY: single-threaded by contract.
    *unsafe { SELECTED.get() } = true;
}

/// Apply the selection once, on the first `shaderc::backend()` call.
///
/// See the module note: the feature is the selection, and a `host-test`
/// build never auto-selects.
pub fn select_once() {
    select(wanted());
}

/// [`select_once`] with the build's answer to "install this back end?"
/// passed in, so a test can run the product's path.
fn select(install: bool) {
    // SAFETY: single-threaded by contract.
    let done = unsafe { SELECTED.get() };
    if *done {
        return;
    }
    *done = true;
    if install {
        // SAFETY: a plain store; no reference to the static is formed.
        unsafe { crate::shaderc::g_shaderc = &SHC_ONLINE };
    }
}

/// Whether to install this back end: always, except in a `host-test` build,
/// whose tests install their own back ends.
fn wanted() -> bool {
    !cfg!(feature = "host-test")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::String;
    use std::vec::Vec;

    use super::store::test_lock;

    /// A shader this compiler made is not a row of the offline table: its
    /// `priv` points at its record in the keep region, and the table's
    /// reader answers None for it (and "" for its name) instead of reading
    /// the record as a row. A real row is found; a pointer into a row is not.
    #[test]
    #[cfg(not(feature = "no-shader-table"))]
    fn a_compiled_shader_is_not_read_as_a_table_row() {
        let _serial = test_lock();
        let src = b"precision mediump float; void main(){ gl_FragColor = vec4(0.75); } // not a row\n\0";
        let mut sh = ShcShader::ZERO;
        // SAFETY: a NUL-terminated source and a real output; no log.
        let rc = unsafe {
            online_compile(SHC_STAGE_FRAGMENT, src.as_ptr() as *const c_char, &mut sh, core::ptr::null_mut(), 0)
        };
        assert_eq!(rc, 0);
        assert!(!sh.priv_.is_null() && store::in_keep(sh.priv_ as *const u8), "the record is in the keep region");
        assert!(crate::shaderc::table_row_of(&sh).is_none());
        assert_eq!(crate::shaderc::shader_source_name(&sh), "");
        release(&sh);
        let row = &crate::shaderc::table::SHC_BLOB_TABLE[0];
        let mut t = row.shader;
        t.priv_ = row as *const crate::shaderc::ShcBlob as *const core::ffi::c_void;
        assert!(core::ptr::eq(crate::shaderc::table_row_of(&t).expect("a row"), row));
        t.priv_ = (t.priv_ as usize + 8) as *const core::ffi::c_void;
        assert!(crate::shaderc::table_row_of(&t).is_none(), "inside a row is not a row");
    }

    /// A back end registered before the first `shaderc::backend()` call is
    /// kept: the selection that installs this back end runs only when
    /// nothing was registered. Without the registration counting, an image
    /// with this compiler could not select the table at all (the first GL
    /// call put this back end over it), which the host harness's A/B hit.
    #[test]
    #[cfg(not(feature = "no-shader-table"))]
    fn an_explicit_registration_survives_the_first_selection() {
        let _serial = test_lock();
        // SAFETY: single-threaded under the test lock; both put back below.
        let (was_selected, was) = unsafe { (*SELECTED.get(), crate::shaderc::g_shaderc) };
        // SAFETY: as above.
        unsafe { *SELECTED.get() = false };
        // SAFETY: as above.
        unsafe { crate::shaderc::offline::shc_offline_register() };
        select(true);
        // SAFETY: as above.
        let kept = unsafe { crate::shaderc::g_shaderc };
        assert!(core::ptr::eq(kept, &crate::shaderc::offline::SHC_OFFLINE), "the registered table is kept");
        /* the control: nothing registered, the selection installs this one */
        // SAFETY: as above.
        unsafe { *SELECTED.get() = false };
        select(true);
        // SAFETY: as above.
        let installed = unsafe { crate::shaderc::g_shaderc };
        assert!(core::ptr::eq(installed, &SHC_ONLINE), "with no registration the compiler is installed");
        // SAFETY: as above.
        unsafe { (*SELECTED.get(), crate::shaderc::g_shaderc) = (was_selected, was) };
    }

    /// Run `online_compile` on `text`; the code and the log come back.
    fn compile(stage: u32, text: &[u8]) -> (i32, String) {
        let mut src = Vec::from(text);
        src.push(0);
        let mut log = [0x55 as c_char; 512];
        let mut out = ShcShader::ZERO;
        // SAFETY: a NUL-terminated source, a real output, and a log of the
        // announced size.
        let rc = unsafe { online_compile(stage, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        // SAFETY: the writer NUL-terminated the log.
        let text = unsafe { cstr_bytes(log.as_ptr()) };
        (rc, String::from_utf8_lossy(text).into_owned())
    }

    #[test]
    fn a_null_source_is_refused_and_says_so() {
        let _serial = test_lock();
        let mut log = [0x55 as c_char; 128];
        let mut out = ShcShader::ZERO;
        // SAFETY: a null source with a valid output and log.
        let rc = unsafe { online_compile(SHC_STAGE_VERTEX, core::ptr::null(), &mut out, log.as_mut_ptr(), log.len()) };
        assert_eq!(rc, -EINVAL);
        // SAFETY: the writer NUL-terminated the log.
        let text = unsafe { cstr_bytes(log.as_ptr()) };
        assert!(String::from_utf8_lossy(text).contains("NULL"));
    }

    /// On the PowerVR a fragment shader the USC half cannot build yet (here
    /// an output written inside a kept loop) says so as -ENOTSUP naming the gap: "this build cannot" and
    /// "your shader is invalid" send a reader to different places.
    #[cfg(feature = "gpu-powervr")]
    #[test]
    fn a_powervr_fragment_shader_the_back_end_cannot_build_is_a_named_gap_and_not_a_rejection() {
        let _serial = test_lock();
        let (rc, log) = compile(
            SHC_STAGE_FRAGMENT,
            b"precision mediump float; uniform int n; void main(){ for (int i = 0; i < 4; i++) { if (i >= n) break; gl_FragColor = vec4(1.0); } }",
        );
        assert_eq!(rc, -ENOTSUP, "{log}");
        assert!(log.contains("an output written inside a loop"), "the log names the missing part: {log}");
        assert!(log.contains("fragment"), "and the stage: {log}");
    }

    /// The fragment shader dEQP-GLES3's texture.shadow group builds
    /// (glsTextureTestUtil.cpp's template, PROGRAM_2D_SHADOW, mediump)
    /// compiles through `online_compile` on the PowerVR: sampled depth,
    /// then compared in the shader (powervr/select.rs shadow_compare).
    /// Until the shadow arm it was refused ("... a shadow compare ... is not
    /// selected yet"), 288 of the AM62P's 432 texture.shadow failures
    /// (RESULTS 1r). What it computes is checked at the selection level
    /// (select.rs a_shadow_look_up_compares_the_reference_with_the_depth_by_the_samplers_function);
    /// this checks the whole chain takes it, down to the USC encoding.
    #[cfg(not(feature = "report-es2"))] // GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[cfg(feature = "gpu-powervr")]
    // the PowerVR back end only: the GC7000 and V3D builds refuse the shadow look-up (main red in gpu-vivante,shaderc-online since 564c9479)
    #[test]
    fn a_powervr_shadow_look_up_compiles_as_deqp_builds_it() {
        let _serial = test_lock();
        let (rc, log) = compile(
            SHC_STAGE_FRAGMENT,
            b"#version 300 es
layout(location = 0) out mediump vec4 dEQP_FragColor;
in mediump vec2 v_texCoord;
uniform mediump float u_bias;
uniform mediump float u_ref;
uniform mediump vec4 u_colorScale;
uniform mediump vec4 u_colorBias;
uniform mediump sampler2DShadow u_sampler;

void main (void)
{
    dEQP_FragColor = vec4(texture(u_sampler, vec3(v_texCoord, u_ref)), 0.0, 0.0, 1.0) * u_colorScale + u_colorBias;
}
",
        );
        assert_eq!(rc, 0, "the shadow look-up compiles: {log}");
    }

    /// A USC pair with a varying links through the entry `glLinkProgram`
    /// reaches, and the link gives the fragment varying the vertex shader's
    /// output dword (a vec4 after gl_Position: 4) -- the number the driver's
    /// coefficient program reads.
    #[cfg(feature = "gpu-powervr")]
    #[test]
    fn a_powervr_pair_with_a_varying_links() {
        let _serial = test_lock();
        let compile_to = |stage: u32, text: &[u8], out: &mut ShcShader| {
            let mut src = Vec::from(text);
            src.push(0);
            let mut log = [0 as c_char; 256];
            // SAFETY: a NUL-terminated source, a real output and log.
            let rc = unsafe { online_compile(stage, src.as_ptr() as *const c_char, out, log.as_mut_ptr(), log.len()) };
            // SAFETY: the writer NUL-terminated the log.
            (rc, String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }).into_owned())
        };
        let (mut vs, mut fs) = (ShcShader::ZERO, ShcShader::ZERO);
        let (rc, log) = compile_to(
            SHC_STAGE_VERTEX,
            b"attribute vec4 a_pos; attribute vec4 a_color; varying vec4 v_color; void main(){ gl_Position = a_pos; v_color = a_color; }",
            &mut vs,
        );
        assert_eq!(rc, 0, "{log}");
        let (rc, log) = compile_to(
            SHC_STAGE_FRAGMENT,
            b"precision mediump float; varying vec4 v_color; void main(){ gl_FragColor = v_color; }",
            &mut fs,
        );
        assert_eq!(rc, 0, "{log}");
        let mut log = [0 as c_char; 256];
        // SAFETY: two shaders this back end produced, and a log.
        let rc = unsafe { online_link(&vs, &mut fs, log.as_mut_ptr(), log.len()) };
        // SAFETY: the writer NUL-terminated the log.
        let text = String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }).into_owned();
        assert_eq!(rc, 0, "{text}");
        assert_eq!((vs.vs_output_size, vs.varyings[0].vtxout_off), (8, 4));
        assert_eq!((fs.num_varyings, fs.varyings[0].vtxout_off, fs.uses_w), (1, 4, 1));
    }

    /// And es100_drawstep's pair compiles, through the entry an application
    /// reaches by `glCompileShader`.
    #[cfg(feature = "gpu-powervr")]
    #[test]
    fn the_powervr_drawstep_pair_compiles_through_the_public_entry_point() {
        let _serial = test_lock();
        let (rc, log) = compile(SHC_STAGE_VERTEX, b"attribute vec4 a_pos; void main(){ gl_Position = a_pos; }");
        assert_eq!(rc, 0, "{log}");
        let (rc, log) = compile(SHC_STAGE_FRAGMENT, b"void main(){ gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }");
        assert_eq!(rc, 0, "{log}");
    }

    /// On the GPU the compiler targets, that same shader compiles, through the
    /// entry point an application reaches by `glCompileShader`.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn a_literal_colour_fragment_shader_compiles_through_the_public_entry_point() {
        let _serial = test_lock();
        let (rc, log) = compile(SHC_STAGE_FRAGMENT, b"void main(){ gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }");
        assert_eq!(rc, 0, "{log}");
    }

    /// A source no front end can accept is a rejection, with the front end's
    /// own diagnostic in the log where an application scrapes it.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn a_source_that_is_not_glsl_is_rejected_with_a_reason() {
        let _serial = test_lock();
        let (rc, log) = compile(SHC_STAGE_FRAGMENT, b"void main(){ this is not glsl }");
        assert_eq!(rc, -EINVAL, "a bad shader is EINVAL, not ENOTSUP: {log}");
        assert!(log.starts_with("0:"), "the log keeps glslang's shape: {log}");
        assert!(log.len() > 3, "and says something: {log}");
    }

    #[test]
    fn a_bad_stage_is_reported_as_the_caller_error_it_is() {
        let _serial = test_lock();
        let (rc, log) = compile(9, b"void main(){}");
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("stage 9"), "{log}");
    }

    #[test]
    fn nothing_is_cached_for_a_compile_that_failed() {
        let _serial = test_lock();
        /* A cache that remembered a failure as a shader would hand the next
         * caller a zeroed ShcShader -- a null USC pointer and zero dwords,
         * which the backend would go on to submit. The record is only made
         * after the compile succeeds. */
        cache_clear();
        /* A source no front end accepts, so the failure is the compile's own
         * and not a missing back end. */
        let _ = compile(SHC_STAGE_VERTEX, b"void main(){ this is not glsl }");
        let (used, overflowed, _, _) = stats();
        assert_eq!((used, overflowed), (0, 0), "a failed compile leaves the cache empty");
    }

    #[test]
    fn blend_variant_of_a_foreign_shader_is_refused_rather_than_guessed() {
        let _serial = test_lock();
        /* A shader from another back end (the table, the server) carries a
         * `priv` this module cannot read. Compiling "the source" would mean
         * guessing which source, so it is refused instead. */
        let fs = ShcShader::ZERO;
        let mut out = ShcShader::ZERO;
        // SAFETY: two valid distinct shaders.
        let rc = unsafe { online_blend_variant(&fs, 0x1234, &mut out, core::ptr::null_mut(), 0) };
        assert_eq!(rc, -EINVAL);
    }

    #[test]
    fn blend_key_zero_is_the_shader_itself() {
        let _serial = test_lock();
        /* The GL core asks for key 0 on every unblended draw; answering it
         * must not need a compile or a cache slot. */
        let mut fs = ShcShader::ZERO;
        fs.temps = 7;
        let mut out = ShcShader::ZERO;
        // SAFETY: two valid distinct shaders.
        let rc = unsafe { online_blend_variant(&fs, 0, &mut out, core::ptr::null_mut(), 0) };
        assert_eq!((rc, out.temps), (0, 7));
    }

    #[test]
    fn the_backend_table_is_complete() {
        /* A `None` here is a null function pointer the GL core would call. */
        assert!(SHC_ONLINE.compile.is_some() && SHC_ONLINE.link.is_some() && SHC_ONLINE.blend_variant.is_some());
    }

    #[test]
    fn a_published_shader_points_at_its_own_record_and_its_own_source() {
        let _serial = test_lock();
        /* What blend_variant depends on: `priv` leads back to the source, so
         * a variant is compiled from the text that made the base shader and
         * not from whatever the caller happened to pass last. */
        cache_clear();
        let txn = store::begin_keep().expect("no compile is open");
        let rec = publish(
            txn,
            [7; 32],
            SHC_STAGE_FRAGMENT,
            0,
            b"void main(){}",
            None,
            ShcShader::ZERO,
            crate::shaderc::ShcBlocks::EMPTY,
        )
        .expect("the keep arena has room for one record");
        let back = base_of(&rec.shader).expect("priv leads back to the record");
        assert_eq!(back.source, b"void main(){}");
        assert_eq!(back.sha, [7; 32]);
        assert_eq!(stats().0, 1, "and it is in the cache");
        let sh = rec.shader;
        release(&sh);
        assert!(base_of(&sh).is_none(), "the last release frees the record");
        assert_eq!(stats().0, 0, "and takes it out of the cache");
        cache_clear();
    }

    /// A vertex shader both GPUs' code generators build, different for each
    /// `i` so that the cache cannot answer it.
    fn distinct_vs(i: u32) -> std::string::String {
        std::format!("attribute vec4 a_pos; void main(){{ gl_Position = a_pos + vec4({i}.0); }}")
    }

    /// Run `online_compile` and hand back the shader it made.
    fn compile_out(stage: u32, text: &str) -> ShcShader {
        let mut src = Vec::from(text.as_bytes());
        src.push(0);
        let mut log = [0 as c_char; 256];
        let mut out = ShcShader::ZERO;
        // SAFETY: a NUL-terminated source, a real output, and a log of the
        // announced size.
        let rc = unsafe { online_compile(stage, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
        // SAFETY: the writer NUL-terminated the log.
        let text = unsafe { cstr_bytes(log.as_ptr()) };
        assert_eq!(rc, 0, "{}", String::from_utf8_lossy(text));
        out
    }

    /// THE product defect this pins: a process that compiles and frees for
    /// ever runs in a bounded region. Before the free path every compile in
    /// the process was refused with `Abi(OutOfMemory { left: 0 })` after
    /// about 200 -- 10,000 is fifty times that, and the region in use after
    /// each release is exactly what it was before the compile.
    ///
    /// Checked: taking `store::free_keep` out of `release` fails this at
    /// compile 0's equality.
    #[test]
    fn ten_thousand_compiles_each_released_keep_the_region_flat() {
        let _serial = test_lock();
        cache_clear();
        let before = store::keep_in_use();
        for i in 0..10_000u32 {
            let sh = compile_out(SHC_STAGE_VERTEX, &distinct_vs(i));
            assert_eq!(holders(&sh), Some(1), "compile {i}: the caller is the one holder");
            assert!(store::keep_in_use() > before, "compile {i} kept its code");
            release(&sh);
            assert_eq!(store::keep_in_use(), before, "compile {i}: the region is back where it started");
        }
        assert_eq!(stats().0, 0, "nothing freed is left in the cache");
    }

    /// A uniform block's table rides in the shader's record: the accessor
    /// finds it, each member's row is a row of the shader's own uniform
    /// table, and releasing the shader gives the region back exactly -- the
    /// table was allocated in the record's own keep transaction.
    ///
    /// Revert check: publish `ShcBlocks::EMPTY` instead of the kept table
    /// and the accessor finds no block.
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn a_shaders_block_table_is_kept_with_it_and_freed_with_it() {
        let _serial = test_lock();
        cache_clear();
        let before = store::keep_in_use();
        let src = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
                   layout(std140) uniform Light { vec4 colour; mat3 m; float f[2]; } light;\n\
                   void main() { o = light.colour + vec4(light.m[1], light.f[1]); }";
        let sh = compile_out(SHC_STAGE_FRAGMENT, src);
        let t = crate::shaderc::uniform_blocks(&sh);
        assert_eq!(t.blocks.len(), 1);
        assert_eq!((t.blocks[0].name, t.blocks[0].data_size), ("Light", 16 + 48 + 32));
        let names: std::vec::Vec<(&str, u32)> = t.members_of(&t.blocks[0]).iter().map(|m| (m.name, m.offset)).collect();
        assert_eq!(names, [("Light.colour", 0), ("Light.m", 16), ("Light.f", 64)]);
        for m in t.members {
            let row = sh.uniform_table()[usize::from(m.row)];
            let want = m.name.replacen("Light", "light", 1);
            assert_eq!(crate::util::name_bytes(&row.name), want.as_bytes(), "{} fills its own row", m.name);
        }
        release(&sh);
        assert_eq!(store::keep_in_use(), before, "the table went with the record");
    }

    /// On the V3D the link replaces the program's fragment shader with the
    /// program record, and the GL core reads a program's uniform blocks
    /// from its two shaders, so the program record carries the fragment
    /// stage's block table, rows resolved against its own uniform table,
    /// and gives it back with the record.
    ///
    /// Revert check: publish `ShcBlocks::EMPTY` in `link_program` and the
    /// accessor finds no block on the linked shader.
    #[cfg(feature = "gpu-v3d")]
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn a_v3d_program_carries_the_fragment_block_table() {
        let _serial = test_lock();
        cache_clear();
        let before = store::keep_in_use();
        let vs = "#version 300 es\nlayout(location = 0) in vec4 p;\nvoid main() { gl_Position = p; }";
        let fs_src = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
                      layout(std140) uniform Light { vec4 colour; float f; } light;\n\
                      void main() { o = light.colour * light.f; }";
        let fs = compile_out(SHC_STAGE_FRAGMENT, fs_src);
        let (rc, text, prog) = link(vs, fs);
        assert_eq!(rc, 0, "link: {text}");
        assert_ne!(prog.usc_dw, 0, "the link built the program");
        let t = crate::shaderc::uniform_blocks(&prog);
        assert_eq!(t.blocks.len(), 1, "the program has the fragment stage's block");
        assert_eq!((t.blocks[0].name, t.blocks[0].data_size), ("Light", 32));
        let names: std::vec::Vec<(&str, u32)> = t.members_of(&t.blocks[0]).iter().map(|m| (m.name, m.offset)).collect();
        assert_eq!(names, [("Light.colour", 0), ("Light.f", 16)]);
        for m in t.members {
            let row = prog.uniform_table()[usize::from(m.row)];
            let want = m.name.replacen("Light", "light", 1);
            assert_eq!(crate::util::name_bytes(&row.name), want.as_bytes(), "{} fills its own row", m.name);
        }
        release(&prog);
        assert_eq!(store::keep_in_use(), before, "the table went with the program record");
    }

    /// `LodKey::ret32` on the V3D (a depth texture: R32F, whose TMU return is
    /// 32 bits a channel). A linked program record is sensitive at every
    /// fragment binding a look-up reads; its variant for a key is the fragment
    /// stage compiled with the key, which the link builds into a program of
    /// its own whose look-up asks for the 32-bit output (P1 bit 0) and four
    /// words (P0 0xf for a vec4). The base program keeps 0x3 / 0x4. A key with
    /// any other bit is refused on the V3D, and a vertex shader is sensitive to
    /// nothing. Revert checks: without the key in link_program's compile the
    /// variant's P1 is 0x4; without it in the cache key the variant is the
    /// base program's record.
    #[cfg(feature = "gpu-v3d")]
    #[test]
    fn a_v3d_program_has_a_ret32_variant() {
        let _serial = test_lock();
        cache_clear();
        let before = store::keep_in_use();
        let vs = "attribute vec4 p;\nvarying vec2 v;\nvoid main() { v = p.xy; gl_Position = p; }";
        let fs_src = "precision mediump float;\nvarying vec2 v;\nuniform sampler2D s;\n\
                      void main() { gl_FragColor = texture2D(s, v); }";
        let cfg = |sh: &ShcShader| -> Vec<(u32, u32)> {
            sh.v3d
                .fs
                .uniform_stream()
                .iter()
                .filter(|u| u.contents == 40 || u.contents == 41)
                .map(|u| (u.contents, u.data))
                .collect()
        };
        let (rc, text, prog) = link(vs, compile_out(SHC_STAGE_FRAGMENT, fs_src));
        assert_eq!(rc, 0, "link: {text}");
        assert_eq!(cfg(&prog), [(40, 0x3), (41, 0x4)], "the base program: f16 words, 16-bit output");
        // SAFETY: a linked program record.
        let sense = LodKey::from_bits(unsafe { online_lod_sensitivity(&prog) });
        assert_eq!(sense, LodKey { ret32: 1, ..LodKey::NONE }, "binding 0, the one look-up");
        let vsh = compile_out(SHC_STAGE_VERTEX, vs);
        // SAFETY: a vertex shader record.
        assert_eq!(unsafe { online_lod_sensitivity(&vsh) }, 0, "a vertex shader is sensitive to nothing on the V3D");
        release(&vsh);
        let key = LodKey { ret32: 1, ..LodKey::NONE };
        let mut var = ShcShader::ZERO;
        let (built, refused) = v3d_sampler_variants();
        // SAFETY: the program record and a real output.
        assert_eq!(unsafe { online_sampler_variant(&prog, key.bits(), &mut var, core::ptr::null_mut(), 0) }, 0);
        assert_eq!(v3d_sampler_variants(), (built + 1, refused), "the variant is counted built");
        let (rc, text, keyed) = link(vs, var);
        assert_eq!(rc, 0, "link of the variant: {text}");
        assert_eq!(cfg(&keyed), [(40, 0xf), (41, 0x5)], "the variant: a word per channel, Output Type 32-bit");
        assert_ne!(keyed.priv_, prog.priv_, "a program record of its own");
        let mut bad = ShcShader::ZERO;
        for other in
            [LodKey { mip_free: 1, ..key }, LodKey { mag_switch: 1, ..key }, LodKey { sample_kill: true, ..key }]
        {
            // SAFETY: as above.
            assert_eq!(
                unsafe { online_sampler_variant(&prog, other.bits(), &mut bad, core::ptr::null_mut(), 0) },
                -EINVAL,
                "{other:?}"
            );
        }
        assert_eq!(v3d_sampler_variants(), (built + 1, refused + 3), "each refused key is counted refused");
        release(&keyed);
        release(&prog);
        assert_eq!(store::keep_in_use(), before, "the records went with their holders");
    }

    /// A shader of the offline table has no record here, and the accessor
    /// answers empty for it rather than failing. (A build without the
    /// table, the V3D's, has no such shader to ask about.)
    #[cfg(not(feature = "no-shader-table"))]
    #[test]
    fn a_table_shader_has_no_blocks() {
        let row = &crate::shaderc::table::SHC_BLOB_TABLE[0];
        let mut sh = row.shader;
        sh.priv_ = row as *const _ as *const core::ffi::c_void;
        let t = crate::shaderc::uniform_blocks(&sh);
        assert!(t.blocks.is_empty() && t.members.is_empty());
        assert!(crate::shaderc::uniform_blocks(&ShcShader::ZERO).blocks.is_empty());
    }

    /// The holders are counted, not guessed: a cache hit and a blend variant
    /// of key 0 are copies like any other, and the shader outlives every
    /// release but the last.
    #[test]
    fn every_copy_is_a_holder_and_only_the_last_release_frees() {
        let _serial = test_lock();
        cache_clear();
        let before = store::keep_in_use();
        let text = distinct_vs(1_000_000);
        let a = compile_out(SHC_STAGE_VERTEX, &text);
        let b = compile_out(SHC_STAGE_VERTEX, &text);
        assert_eq!(a.priv_, b.priv_, "the second compile is the cache's answer");
        assert_eq!(holders(&a), Some(2));
        let mut c = ShcShader::ZERO;
        // SAFETY: two valid distinct shaders.
        assert_eq!(unsafe { online_blend_variant(&a, 0, &mut c, core::ptr::null_mut(), 0) }, 0);
        assert_eq!(holders(&a), Some(3), "key 0 hands out a copy, and counts it");
        retain(&a);
        assert_eq!(holders(&a), Some(4));
        for (n, left) in [(&a, 3), (&b, 2), (&c, 1)] {
            release(n);
            assert_eq!(holders(&a), Some(left));
            /* The record is live, so what the shader points into is too: its
             * code, or on the V3D -- where a stage has no code, the linked
             * program has it -- its attribute rows' names. */
            #[cfg(not(feature = "gpu-v3d"))]
            // SAFETY: the record is live, so its code is too.
            assert!(!a.usc.is_null() && unsafe { *a.usc } == unsafe { *b.usc });
            #[cfg(feature = "gpu-v3d")]
            assert!(a.usc_dw == 0 && a.num_attribs > 0 && a.attribs[0].name == b.attribs[0].name);
        }
        release(&a);
        assert_eq!(holders(&a), None, "the last holder freed it");
        assert_eq!(store::keep_in_use(), before);
        /* A shader no back end here made, and a zeroed one, are nothing to
         * release -- the GL core calls this on every shader it holds. */
        release(&ShcShader::ZERO);
        let mut foreign = ShcShader::ZERO;
        foreign.priv_ = &before as *const usize as *const c_void;
        release(&foreign);
        retain(&foreign);
        assert_eq!(store::keep_in_use(), before);
    }

    #[test]
    fn the_cache_reports_its_overflow_instead_of_evicting_silently() {
        let _serial = test_lock();
        let _keep = store::begin_keep().expect("no compile is open"); /* the fake record's memory */
        /* The cache is a bounded array; when it is full the compile still
         * works, and the count says how often that happened -- so "slow" can
         * be told from "too small". */
        cache_clear();
        // SAFETY: single-threaded by contract.
        let c = unsafe { CACHE.get() };
        c.used = CACHE_SLOTS;
        let rec = store::keep()
            .alloc_filled(
                1,
                Compiled {
                    magic: MAGIC,
                    refs: 1,
                    extent: store::KeepExtent::EMPTY,
                    sha: [0; 32],
                    stage: SHC_STAGE_VERTEX,
                    blend_key: 0,
                    source: b"",
                    w_constant: false,
                    lod_key: LodKey::NONE,
                    shader: ShcShader::ZERO,
                    blocks: crate::shaderc::ShcBlocks::EMPTY,
                    attrib_map: None,
                },
            )
            .expect("the keep arena has room for one record");
        let ptr = rec.as_ptr();
        // SAFETY: an ambient keep arena's allocations are kept for the process.
        cache_put(unsafe { &*ptr });
        let (used, overflowed, _, _) = stats();
        assert_eq!((used, overflowed), (CACHE_SLOTS, 1));
        cache_clear();
    }

    /* ---- section 4.3.5 and the uniform rules at link ---- */

    /// A fragment shader record as a code generator would have made it for
    /// `src`: its source, and one input per name in `inputs` -- on the GC7000
    /// in `viv.in_` (the `by_register` pairing), on the PowerVR in
    /// `varyings` (`by_name`). Hand-made, because the PowerVR code generator
    /// does not build a fragment shader that reads a varying yet, and the
    /// link rules must be pinned on both arms regardless.
    fn fs_record(src: &str, inputs: &[&str]) -> ShcShader {
        let txn = store::begin_keep().expect("no compile is open");
        let mut sh = ShcShader::ZERO;
        for (i, n) in inputs.iter().enumerate() {
            #[cfg(feature = "gpu-vivante")]
            {
                crate::util::set_name(&mut sh.viv.in_[i].name, n.as_bytes());
                sh.viv.in_[i].components = 4;
                sh.viv.num_in = inputs.len() as u8;
            }
            #[cfg(not(feature = "gpu-vivante"))]
            {
                crate::util::set_name(&mut sh.varyings[i].name, n.as_bytes());
                sh.varyings[i].components = 4;
                sh.num_varyings = inputs.len() as u8;
            }
        }
        publish(
            txn,
            sha256(src.as_bytes()),
            SHC_STAGE_FRAGMENT,
            0,
            src.as_bytes(),
            None,
            sh,
            crate::shaderc::ShcBlocks::EMPTY,
        )
        .expect("room")
        .shader
    }

    /// `online_link` on a compiled vertex shader and a fragment record; the
    /// code, the log, and the fragment shader the program would now hold.
    fn link(vs_src: &str, fs: ShcShader) -> (i32, String, ShcShader) {
        let vs = compile_out(SHC_STAGE_VERTEX, vs_src);
        let mut fs = fs;
        let mut log = [0 as c_char; 512];
        // SAFETY: two distinct valid shaders and a log of the announced size.
        let rc = unsafe { online_link(&vs, &mut fs, log.as_mut_ptr(), log.len()) };
        // SAFETY: the writer NUL-terminated the log.
        let text = String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }).into_owned();
        release(&vs);
        (rc, text, fs)
    }

    fn fs_inputs(sh: &ShcShader) -> usize {
        #[cfg(feature = "gpu-vivante")]
        return sh.viv.num_in as usize;
        #[cfg(not(feature = "gpu-vivante"))]
        return sh.num_varyings as usize;
    }

    const FS_READS_V: &str =
        "precision mediump float; varying vec4 v; void main(){ gl_FragColor = v + vec4(0.25, 0.5, 0.75, 1.0); }";

    /// GLSL ES 1.00 section 4.3.5: a varying the vertex shader DECLARES and
    /// never writes is legal. The program's fragment shader becomes the same
    /// source recompiled with `v` read as 0 -- no input left for either
    /// back end to wire -- and the caller's reference moves to it. dEQP
    /// `linkage.varying_4` was refused as "not written" until 2026-09-23.
    ///
    /// Checked: with `link_online` returning 0 before the substitution the
    /// table rules refuse this ("is not written by the vertex shader").
    #[test]
    fn a_varying_declared_and_never_written_reads_as_zero() {
        let _serial = test_lock();
        let before = store::keep_in_use();
        let fs = fs_record(FS_READS_V, &["v"]);
        let old = fs.priv_;
        let (rc, log, linked) = link("attribute vec4 p; varying vec4 v; void main(){ gl_Position = p; }", fs);
        assert_eq!(rc, 0, "{log}");
        assert_ne!(linked.priv_, old, "the program holds the substituted shader");
        assert_eq!(fs_inputs(&linked), 0, "and it reads no varying");
        assert_eq!(holders(&linked), Some(1));
        let mut gone = ShcShader::ZERO;
        gone.priv_ = old;
        assert_eq!(holders(&gone), None, "the unsubstituted one was the caller's to give back");
        release(&linked);
        assert_eq!(store::keep_in_use(), before);
    }

    /* ---- a constant W: the fragment shader keeps the TPU's level ---- */

    /// `ir::Module::constant_position_w` is strict: a constant written on
    /// every path, through the locals a `vec4(...)` lowers to; anything
    /// that may vary is `None`.
    #[test]
    fn constant_w_is_a_literal_written_on_every_path() {
        let cases: &[(&str, Option<f32>)] = &[
            ("attribute vec2 p; void main(){ gl_Position = vec4(p, 0.0, 1.0); }", Some(1.0)),
            ("attribute vec2 p; void main(){ gl_Position = vec4(p * 2.0 - 1.0, 0.5, 2.0); }", Some(2.0)),
            ("attribute vec4 p; void main(){ gl_Position = p; gl_Position.w = 1.0; }", Some(1.0)),
            ("attribute vec2 p; void main(){ vec4 q = vec4(p, 0.0, 1.0); gl_Position = q; }", Some(1.0)),
            ("attribute vec4 p; void main(){ gl_Position = p; }", None),
            ("attribute vec4 p; uniform mat4 m; void main(){ gl_Position = m * p; }", None),
            ("attribute vec2 p; uniform float w; void main(){ gl_Position = vec4(p, 0.0, w); }", None),
            (
                "attribute vec2 p; uniform float s; void main(){ gl_Position = vec4(p, 0.0, 1.0); \
                 if (s > 2.0) gl_Position.w = 2.0; }",
                None,
            ),
            (
                "attribute vec2 p; uniform float s; void main(){ vec4 q = vec4(p, 0.0, 1.0); \
                 if (s > 2.0) q.w = 2.0; gl_Position = q; }",
                None,
            ),
            (
                "attribute vec2 p; uniform float s; void main(){ if (s > 2.0) { gl_Position = vec4(p, 0.0, 1.0); return; } \
                 gl_Position = vec4(p, 0.0, 1.0); }",
                None,
            ),
        ];
        for (src, want) in cases {
            let got = store::with_scratch(|a| {
                let text = a.alloc_str(src).unwrap();
                let l = super::super::lower::compile_to_ir(a, super::super::sema::Stage::Vertex, text).expect("lowers");
                l.module.constant_position_w()
            })
            .unwrap();
            assert_eq!(got.map(f32::from_bits), *want, "{src}");
        }
    }

    #[cfg(feature = "gpu-powervr")]
    const FS_SAMPLES_UV: &str =
        "precision mediump float; uniform sampler2D s; varying vec2 uv; void main(){ gl_FragColor = texture2D(s, uv); }";

    #[cfg(feature = "gpu-powervr")]
    fn code_of(sh: &ShcShader) -> Vec<u32> {
        // SAFETY: a compiled shader's code, `usc_dw` dwords in keep.
        unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }.to_vec()
    }

    /// A program whose vertex shader writes a constant W links to the
    /// fragment program `pvr_proj_lod = 0` compiles, dword for dword, and its
    /// blend variants likewise; a vertex shader whose W varies keeps the
    /// gradients (`shaderc::pvr_proj_lod`, `link_online`).
    #[cfg(feature = "gpu-powervr")]
    #[test]
    fn a_constant_w_program_samples_as_the_switch_off_does() {
        let _serial = test_lock();
        let key = crate::shaderc::SHC_BLEND_KEY_REPLACE | crate::shaderc::shc_blend_key_mask_off(8);
        let variant = |fs: &ShcShader| {
            let mut v = ShcShader::ZERO;
            // SAFETY: a compiled shader and a real output.
            assert_eq!(unsafe { online_blend_variant(fs, key, &mut v, core::ptr::null_mut(), 0) }, 0);
            v
        };
        /* the switch-off programs, released so the cache (keyed by the text)
         * does not hand them back below */
        crate::shaderc::pvr_set_proj_lod(0);
        let off = compile_out(SHC_STAGE_FRAGMENT, FS_SAMPLES_UV);
        let off_var = variant(&off);
        let (off_code, off_var_code) = (code_of(&off), code_of(&off_var));
        release(&off_var);
        release(&off);
        crate::shaderc::pvr_set_proj_lod(crate::shaderc::PVR_PROJ_LOD_DEFAULT);

        let fs = compile_out(SHC_STAGE_FRAGMENT, FS_SAMPLES_UV);
        let grad_code = code_of(&fs);
        assert_ne!(grad_code, off_code, "the default samples with gradients");
        /* what the link-time compile costs, printed (--nocapture): the
         * second record's keep extent and the link's host time */
        let t0 = std::time::Instant::now();
        let (rc, log, linked) =
            link("attribute vec2 p; varying vec2 uv; void main(){ uv = p; gl_Position = vec4(p, 0.0, 1.0); }", fs);
        let bytes = base_of(&linked).map_or(0, |r| r.extent.bytes());
        std::println!("  constant-W link: a record of {bytes} keep bytes, {:?} (host)", t0.elapsed());
        assert_eq!(rc, 0, "{log}");
        assert_eq!(code_of(&linked), off_code, "constant W: the switch-off program");
        let var = variant(&linked);
        assert_eq!(code_of(&var), off_var_code, "and the switch-off blend variant");
        release(&var);
        release(&linked);

        for vs in [
            "attribute vec4 p; varying vec2 uv; void main(){ uv = p.xy; gl_Position = p; }",
            "attribute vec2 p; uniform float w; varying vec2 uv; void main(){ uv = p; gl_Position = vec4(p, 0.0, w); }",
        ] {
            let fs = compile_out(SHC_STAGE_FRAGMENT, FS_SAMPLES_UV);
            let (rc, log, linked) = link(vs, fs);
            assert_eq!(rc, 0, "{log}");
            assert_eq!(code_of(&linked), grad_code, "W varies: the gradients stay\n{vs}");
            let var = variant(&linked);
            assert_ne!(code_of(&var), off_var_code, "and in the blend variant\n{vs}");
            release(&var);
            release(&linked);
        }
    }

    /* ---- sampler-state variants (shaderc::LodKey) ---- */

    /// The C ABI's `uint64_t` of a [`LodKey`]: `mip_free` bits 0..15,
    /// `ret32` 16..31, `mag_switch` 32..62, `sample_kill` 63. Bindings 16
    /// and up of `mip_free` and `ret32` are not carried: the GL core keys
    /// bindings below CAP_SAMPLERS, 16, which this test also pins (a larger
    /// CAP_SAMPLERS needs another layout).
    #[test]
    fn the_lod_key_layout_is_pinned() {
        let n = LodKey::NONE;
        assert_eq!(LodKey { mip_free: 1, ..n }.bits(), 1);
        assert_eq!(LodKey { ret32: 1, ..n }.bits(), 1 << 16);
        assert_eq!(LodKey { ret32: 0x8000, ..n }.bits(), 1 << 31);
        assert_eq!(LodKey { mag_switch: 1, ..n }.bits(), 1 << 32);
        assert_eq!(LodKey { sample_kill: true, ..n }.bits(), 1 << 63);
        let k = LodKey { mip_free: 0x8001, ret32: 0x4002, mag_switch: 0x7fff_ffff, sample_kill: true };
        assert_eq!(LodKey::from_bits(k.bits()), k);
        assert_eq!(LodKey { mip_free: 1 << 16, ret32: 1 << 16, ..n }.bits(), 0, "bindings 16 and up are not keyed");
        assert!(!LodKey { ret32: 1, ..n }.is_none() && n.is_none());
        assert_eq!(crate::gl::context::CAP_SAMPLERS, 16, "the bindings the key carries");
    }

    #[cfg(not(feature = "gpu-v3d"))]
    fn sensitivity(sh: &ShcShader) -> LodKey {
        // SAFETY: a valid shader.
        LodKey::from_bits(unsafe { online_lod_sensitivity(sh) })
    }

    #[cfg(not(feature = "gpu-v3d"))]
    fn sampler_variant(sh: &ShcShader, key: LodKey) -> ShcShader {
        let mut v = ShcShader::ZERO;
        // SAFETY: a compiled shader and a real output.
        assert_eq!(unsafe { online_sampler_variant(sh, key.bits(), &mut v, core::ptr::null_mut(), 0) }, 0, "{key:?}");
        v
    }

    #[cfg(not(feature = "gpu-v3d"))]
    const VS_CUBE_LOD: &str = "attribute vec4 a; uniform highp float u_lod; uniform samplerCube s; varying vec4 v; \
                               void main(){ v = textureCubeLod(s, a.xyz, u_lod); gl_Position = a; }";

    /// Which bindings each shader's code depends on: a fragment look-up
    /// with exact gradients (PowerVR) is `mip_free`, an explicit level
    /// `mag_switch`; a shader without either, or with the switches off,
    /// has none; a foreign shader has none.
    /* the V3D compiles programs whole at link: no variants (sensitivity_of) */
    #[cfg(not(feature = "gpu-v3d"))]
    #[test]
    fn the_sensitivity_names_the_look_ups_the_key_can_change() {
        let _serial = test_lock();
        let vs = compile_out(SHC_STAGE_VERTEX, VS_CUBE_LOD);
        /* the remap is for an ES 2.0 report only (`sensitivity_of`) */
        let es2 = u32::from(crate::REPORT_ES2);
        assert_eq!(sensitivity(&vs), LodKey { mip_free: 0, mag_switch: es2, sample_kill: false, ret32: 0 });
        let plain = compile_out(SHC_STAGE_FRAGMENT, FS_READS_V);
        /* the PowerVR's fragment shaders are all sensitive to the sample
         * state (`LodKey::sample_kill`) */
        let kill = SAMPLE_KILL_VARIANTS;
        assert_eq!(sensitivity(&plain), LodKey { sample_kill: kill, ..LodKey::NONE });
        #[cfg(feature = "gpu-powervr")]
        {
            let fs = compile_out(SHC_STAGE_FRAGMENT, FS_SAMPLES_UV);
            assert_eq!(sensitivity(&fs), LodKey { mip_free: 1, mag_switch: 0, sample_kill: true, ret32: 0 });
            crate::shaderc::shc_set_lod_variants(0, -1);
            assert_eq!(sensitivity(&fs), LodKey { sample_kill: true, ..LodKey::NONE }, "switch (a) off");
            crate::shaderc::shc_set_lod_variants(1, -1);
            release(&fs);
        }
        crate::shaderc::shc_set_lod_variants(-1, 0);
        assert_eq!(sensitivity(&vs), LodKey::NONE, "switch (b) off");
        crate::shaderc::shc_set_lod_variants(-1, 1);
        assert_eq!(sensitivity(&ShcShader::ZERO), LodKey::NONE, "not this back end's");
        release(&plain);
        release(&vs);
    }

    /// The sample-kill variant (`LodKey::sample_kill`) of a fragment shader
    /// with no `discard` carries the epilogue a discarding shader has: the
    /// record says it discards and where its control word goes, and its
    /// blend variant keeps it. A shader that discards is not sensitive: it
    /// has the epilogue already.
    #[cfg(feature = "gpu-powervr")]
    #[test]
    fn a_sample_kill_variant_carries_the_discard_epilogue() {
        let _serial = test_lock();
        let fs = compile_out(SHC_STAGE_FRAGMENT, FS_READS_V);
        assert_eq!((fs.uses_discard, fs.meta_off), (0, crate::shaderc::SHC_NO_META));
        let kill = LodKey { sample_kill: true, ..LodKey::NONE };
        let v = sampler_variant(&fs, kill);
        assert_eq!(v.uses_discard, 1, "the epilogue's kill");
        assert_ne!(v.meta_off, crate::shaderc::SHC_NO_META, "and its control word");
        assert_ne!(code_of(&v), code_of(&fs));
        assert_eq!(v.num_varyings, fs.num_varyings);
        let key = crate::shaderc::shc_blend_key(
            crate::shaderc::SHC_BF_ONE,
            crate::shaderc::SHC_BF_ONE,
            crate::shaderc::SHC_BF_ONE,
            crate::shaderc::SHC_BF_ONE,
            crate::shaderc::SHC_BLEND_ADD,
            crate::shaderc::SHC_BLEND_ADD,
        );
        let mut vb = ShcShader::ZERO;
        // SAFETY: a compiled shader and a real output.
        assert_eq!(unsafe { online_blend_variant(&v, key, &mut vb, core::ptr::null_mut(), 0) }, 0);
        assert_eq!(vb.uses_discard, 1, "the blend variant inherits the key");
        release(&vb);
        release(&v);
        release(&fs);

        let d = compile_out(
            SHC_STAGE_FRAGMENT,
            "precision mediump float; varying vec4 v; void main(){ if (v.x < 0.5) discard; gl_FragColor = v; }",
        );
        assert_eq!(d.uses_discard, 1);
        assert!(!sensitivity(&d).sample_kill, "a discarding shader has the epilogue");
        release(&d);
    }

    /// Switch (a): the variant of a fragment shader for a sampler with no
    /// mipmap filter is the program `pvr_proj_lod = 0` compiles, dword for
    /// dword, and so are its blend variant and its link; key 0 is the input
    /// itself. The variant keeps its interface (varyings, uniforms).
    #[cfg(feature = "gpu-powervr")]
    #[test]
    fn a_mip_free_variant_is_the_program_without_gradients() {
        let _serial = test_lock();
        let key = crate::shaderc::SHC_BLEND_KEY_REPLACE | crate::shaderc::shc_blend_key_mask_off(8);
        let blend = |fs: &ShcShader| {
            let mut v = ShcShader::ZERO;
            // SAFETY: a compiled shader and a real output.
            assert_eq!(unsafe { online_blend_variant(fs, key, &mut v, core::ptr::null_mut(), 0) }, 0);
            v
        };
        crate::shaderc::pvr_set_proj_lod(0);
        let off = compile_out(SHC_STAGE_FRAGMENT, FS_SAMPLES_UV);
        let off_blend = blend(&off);
        let (off_code, off_blend_code) = (code_of(&off), code_of(&off_blend));
        release(&off_blend);
        release(&off);
        crate::shaderc::pvr_set_proj_lod(crate::shaderc::PVR_PROJ_LOD_DEFAULT);

        let fs = compile_out(SHC_STAGE_FRAGMENT, FS_SAMPLES_UV);
        assert_ne!(code_of(&fs), off_code, "the base samples with gradients");
        let same = sampler_variant(&fs, LodKey::NONE);
        assert_eq!(same.usc, fs.usc, "key 0: the input");
        release(&same);
        let v = sampler_variant(&fs, LodKey { mip_free: 1, mag_switch: 0, sample_kill: false, ret32: 0 });
        assert_eq!(code_of(&v), off_code, "mip-free: the switch-off program");
        assert_eq!(v.num_varyings, fs.num_varyings);
        assert_eq!(v.uniform_dwords, fs.uniform_dwords);
        assert_eq!(v.num_uniforms, fs.num_uniforms);
        let vb = blend(&v);
        assert_eq!(code_of(&vb), off_blend_code, "and its blend variant");
        release(&vb);
        /* linked to a vertex shader whose W varies: still no gradients */
        let (rc, log, linked) =
            link("attribute vec4 p; varying vec2 uv; void main(){ uv = p.xy; gl_Position = p; }", v);
        assert_eq!(rc, 0, "{log}");
        assert_eq!(code_of(&linked), off_code, "the link keeps the key");
        release(&linked);
        /* a key for a binding the shader does not read: the gradients stay */
        let other = sampler_variant(&fs, LodKey { mip_free: 2, mag_switch: 0, sample_kill: false, ret32: 0 });
        assert_eq!(code_of(&other), code_of(&fs));
        release(&other);
        release(&fs);
    }

    /// Switch (b): the variant of a vertex shader with an explicit-level
    /// look-up has the level remapped (three more IR instructions: the
    /// code grows), keeps its interface, and keeps the attribute map of a
    /// shader compiled at link.
    /* the V3D compiles programs whole at link: no variants (sensitivity_of) */
    #[cfg(not(feature = "gpu-v3d"))]
    #[test]
    fn a_mag_switch_variant_remaps_the_level_and_keeps_the_layout() {
        let _serial = test_lock();
        let vs = compile_out(SHC_STAGE_VERTEX, VS_CUBE_LOD);
        let v = sampler_variant(&vs, LodKey { mip_free: 0, mag_switch: 1, sample_kill: false, ret32: 0 });
        assert_ne!(v.usc_words(), vs.usc_words(), "the remap is code");
        /* what it costs this GPU (--nocapture) */
        std::println!(
            "  mag-switch remap: code dwords {} -> {}, temps {} -> {}",
            vs.usc_dw,
            v.usc_dw,
            vs.temps,
            v.temps
        );
        assert_eq!(v.num_uniforms, vs.num_uniforms);
        assert_eq!(v.uniform_dwords, vs.uniform_dwords);
        assert_eq!(v.num_attribs, vs.num_attribs);
        /* the same key twice: the cached record */
        let again = sampler_variant(&vs, LodKey { mip_free: 0, mag_switch: 1, sample_kill: false, ret32: 0 });
        assert_eq!(again.usc, v.usc);
        release(&again);
        release(&v);

        /* a vertex shader compiled with the link's layout: the variant keeps it */
        let mut map = crate::shaderc::ShcAttribMap::ZERO;
        map.count = 1;
        map.b[0].name[0] = b'a' as _;
        map.b[0].location = 3;
        map.b[0].columns = 1;
        map.b[0].gl_type = 0x8B52; /* vec4 */
        map.b[0].active = 1;
        let src = std::format!("{VS_CUBE_LOD}\0");
        let mut linked = ShcShader::ZERO;
        let mut log = [0 as c_char; 256];
        // SAFETY: a NUL-terminated source, a map, a real output and a log.
        let rc = unsafe {
            online_compile_vs_linked(src.as_ptr() as *const c_char, &map, &mut linked, log.as_mut_ptr(), log.len())
        };
        assert_eq!(rc, 0);
        let lv = sampler_variant(&linked, LodKey { mip_free: 0, mag_switch: 1, sample_kill: false, ret32: 0 });
        let loc = |s: &ShcShader| s.attribs[..s.num_attribs as usize].iter().map(|a| a.location).collect::<Vec<_>>();
        assert_eq!(loc(&lv), loc(&linked), "the map's locations");
        assert!(loc(&linked).contains(&3));
        release(&lv);
        release(&linked);
        release(&vs);
    }

    /* ---- the varying budget (GLSL ES 1.00 Appendix A 7; reflect::varying_budget) ---- */

    /// A pair sharing `decls` (`(type, name, array length or 0)`): the
    /// vertex shader writes every one, the fragment shader reads those in
    /// `read` (and `extra`, a built-in's use, into the colour).
    fn varying_pair(decls: &[(&str, String, u32)], read: impl Fn(&str) -> bool, extra: &str) -> (String, String) {
        let decl = |(ty, name, n): &(&str, String, u32)| {
            if *n == 0 {
                std::format!("varying {ty} {name};")
            } else {
                std::format!("varying {ty} {name}[{n}];")
            }
        };
        /* from the attribute, so each write is one consecutive register run
         * (the PowerVR vertex epilogue has room for 32 write groups, and a
         * constant per component can take one each) */
        let one = |ty: &str| match ty {
            "float" => "p.x",
            "vec2" => "p.xy",
            "vec3" => "p.xyz",
            "mat2" => "mat2(p)",
            "mat3" => "mat3(p.xyz, p.xyz, p.xyz)",
            "mat4" => "mat4(p, p, p, p)",
            _ => "p",
        };
        let first = |ty: &str, x: &str| match ty {
            "float" => x.to_string(),
            t if t.starts_with("mat") => std::format!("{x}[0][0]"),
            _ => std::format!("{x}.x"),
        };
        let mut vs = String::from("attribute vec4 p;\n");
        let mut fs = String::from("precision mediump float;\n");
        let (mut writes, mut reads) = (String::new(), String::from("float acc = 0.0;"));
        for d in decls {
            vs.push_str(&(decl(d) + "\n"));
            fs.push_str(&(decl(d) + "\n"));
            let (ty, name, n) = d;
            let elems: Vec<String> =
                if *n == 0 { std::vec![name.clone()] } else { (0..*n).map(|k| std::format!("{name}[{k}]")).collect() };
            for e in &elems {
                writes.push_str(&std::format!(" {e} = {};", one(ty)));
            }
            if read(name) {
                reads.push_str(&std::format!(" acc += {};", first(ty, &elems[0])));
            }
        }
        vs.push_str(&std::format!("void main(){{ gl_Position = p;{writes} }}\n"));
        fs.push_str(&std::format!("void main(){{ {reads} gl_FragColor = vec4(acc){extra}; }}\n"));
        (vs, fs)
    }

    fn vec4s(n: usize) -> Vec<(&'static str, String, u32)> {
        (0..n).map(|i| ("vec4", std::format!("v{i}"), 0)).collect()
    }

    /// `online_link` on the two sources, both compiled by this back end.
    fn link_sources(vs: &str, fs: &str) -> (i32, String) {
        let fs = compile_out(SHC_STAGE_FRAGMENT, fs);
        let (rc, log, fs) = link(vs, fs);
        release(&fs);
        (rc, log)
    }

    /// A vertex shader whose constant file is `uniforms` vec4 uniform
    /// registers plus about `literals` registers of literals: each literal
    /// is a vec4 of four values no other literal has.
    #[cfg(feature = "gpu-vivante")]
    fn constant_heavy_vs(uniforms: usize, literals: usize) -> String {
        let mut s = std::format!("attribute vec4 p;\nuniform vec4 u[{uniforms}];\nvoid main(){{ vec4 r = p;");
        for k in 0..uniforms {
            s.push_str(&std::format!(" r += u[{k}];"));
        }
        for k in 0..literals {
            let b = 4 * k + 2;
            s.push_str(&std::format!(" r *= vec4({}.25, {}.25, {}.25, {}.25);", b, b + 1, b + 2, b + 3));
        }
        s.push_str(" gl_Position = r; }\n");
        s
    }

    /// dEQP uniform_api.random.3's shape, without its thirteen vertex
    /// samplers: 174 registers of uniforms and 101 of literals, 275 of the
    /// GC7000's 320. The program links, and its vertex constant file really
    /// is past register 255 (the draw writes all of it:
    /// `a_vertex_constant_file_past_256_registers_is_written_whole`).
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn a_vertex_shader_with_more_than_256_constant_registers_links() {
        let _serial = test_lock();
        let vs = constant_heavy_vs(174, 100);
        let compiled = compile_out(SHC_STAGE_VERTEX, &vs);
        let regs = crate::vivante::shader::constant_registers(&compiled);
        release(&compiled);
        assert!((257..=crate::vivante::regs::GC_NUM_CONSTANTS).contains(&regs), "{regs} registers");
        let (rc, log) = link_sources(&vs, "precision mediump float;\nvoid main(){ gl_FragColor = vec4(1.0); }\n");
        assert_eq!(rc, 0, "{log}");
    }

    /// Past the 320 registers the two stages share, the link refuses and says
    /// why, rather than a draw dropping the registers that do not fit.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn a_program_whose_constant_files_exceed_the_gc7000_file_is_refused_at_link() {
        let _serial = test_lock();
        let vs = constant_heavy_vs(250, 80);
        let (rc, log) = link_sources(&vs, "precision mediump float;\nvoid main(){ gl_FragColor = vec4(1.0); }\n");
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("exceed the 320 this GPU has"), "{log}");
    }

    fn varying_budget_message() -> String {
        std::format!(
            "online shaderc: link error: the program's varyings do not fit MAX_VARYING_VECTORS = {} (Appendix A packing)",
            crate::gl::context::MAX_VARYING_VECTORS
        )
    }

    const ROWS: usize = crate::gl::context::MAX_VARYING_VECTORS;

    #[test]
    fn varyings_that_fill_max_varying_vectors_link() {
        let _serial = test_lock();
        let (vs, fs) = varying_pair(&vec4s(ROWS), |_| true, "");
        let (rc, log) = link_sources(&vs, &fs);
        assert_eq!(rc, 0, "{log}");
    }

    /// The GC7000 code generator refuses a vertex shader with 16 outputs
    /// before any link (`Abi(LoadBalancingTempsUnmeasured)`, pinned by
    /// `a_vertex_shader_with_sixteen_outputs_is_refused_by_the_gc7000_code_generator`),
    /// so the four grid-filling link tests run on the other GPUs only.
    #[cfg(not(feature = "gpu-vivante"))]
    #[test]
    fn one_varying_over_max_varying_vectors_fails_the_link() {
        let _serial = test_lock();
        let (vs, fs) = varying_pair(&vec4s(ROWS + 1), |_| true, "");
        assert_eq!(link_sources(&vs, &fs), (-EINVAL, varying_budget_message()));
    }

    /// Appendix A: "mat2 occupies 2 complete rows". ROWS / 2 mat2 fill the
    /// grid (all of it when ROWS is even, all but one row when it is odd),
    /// so a float array of ROWS % 2 + 1 rows more fails -- where ROWS / 2
    /// vec2[2], the same components, would leave half of every row. (16
    /// outputs again: PowerVR/V3D only, see the GC7000 pin below.)
    #[cfg(not(feature = "gpu-vivante"))]
    #[test]
    fn mat2_varyings_take_full_rows() {
        let _serial = test_lock();
        let mut decls: Vec<(&str, String, u32)> = (0..ROWS / 2).map(|i| ("mat2", std::format!("m{i}"), 0)).collect();
        decls.push(("float", "f".into(), ROWS as u32 % 2 + 1));
        let (vs, fs) = varying_pair(&decls, |_| true, "");
        assert_eq!(link_sources(&vs, &fs), (-EINVAL, varying_budget_message()));
    }

    /// Appendix A: "Elements of an array must be in different rows": a
    /// float[ROWS + 1] fits no column, although its ROWS + 1 components are
    /// far fewer than the grid's 4 * ROWS.
    #[cfg(not(feature = "gpu-vivante"))]
    #[test]
    fn a_float_array_longer_than_the_grid_fails() {
        let _serial = test_lock();
        let (vs, fs) = varying_pair(&[("float", "f".into(), ROWS as u32 + 1)], |_| true, "");
        assert_eq!(link_sources(&vs, &fs), (-EINVAL, varying_budget_message()));
    }

    /// "If referenced in the fragment shader [...] the built-in special
    /// variables (gl_FragCoord, gl_FrontFacing and gl_PointCoord) are
    /// included": a full grid plus gl_FragCoord does not fit.
    #[test]
    fn a_referenced_fragment_builtin_counts_as_a_varying() {
        let _serial = test_lock();
        let (vs, fs) = varying_pair(&vec4s(ROWS), |_| true, " + gl_FragCoord");
        assert_eq!(link_sources(&vs, &fs), (-EINVAL, varying_budget_message()));
    }

    /// "Only varyings statically used in both shaders are counted": one
    /// varying past the grid that the fragment shader declares and never
    /// reads is not.
    #[cfg(not(feature = "gpu-vivante"))]
    #[test]
    fn a_varying_the_fragment_shader_never_uses_is_not_counted() {
        let _serial = test_lock();
        let (vs, fs) = varying_pair(&vec4s(ROWS + 1), |name| name != "v0", "");
        let (rc, log) = link_sources(&vs, &fs);
        assert_eq!(rc, 0, "{log}");
    }

    /// The GC7000 code generator's limit comes before the Appendix A check:
    /// a vertex shader with ROWS + 1 = 16 vec4 outputs is refused at compile
    /// as a code-generator gap (-ENOTSUP), not as a malformed shader. Until
    /// the generator takes 16 outputs, the four grid-filling link tests above
    /// are PowerVR/V3D only.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn a_vertex_shader_with_sixteen_outputs_is_refused_by_the_gc7000_code_generator() {
        let _serial = test_lock();
        let (vs, _fs) = varying_pair(&vec4s(ROWS + 1), |_| true, "");
        let mut src = Vec::from(vs.as_bytes());
        src.push(0);
        let mut log = [0 as c_char; 256];
        let mut out = ShcShader::ZERO;
        // SAFETY: a NUL-terminated source, a real output, and a log of the
        // announced size.
        let rc = unsafe {
            online_compile(SHC_STAGE_VERTEX, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len())
        };
        // SAFETY: the writer NUL-terminated the log.
        let text = String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }).into_owned();
        assert_eq!(rc, -crate::util::ENOTSUP, "{text}");
        assert!(text.contains("LoadBalancingTempsUnmeasured"), "{text}");
    }

    /// gl_test 149's vertex shader (vsunits) with 9..16 samplers, as the
    /// GC7000's code generator builds it: one texldl per sampler, TEX_ID the
    /// sampler's own stage-relative binding 0..n-1 (the backend adds
    /// VS_SAMPLER_BASE; Mesa etnaviv_compiler_nir_emit.c:210-214, unified
    /// samplers), and one sampler binding per u_s<k>. The compiler does not
    /// read the sixteen-unit switch; the link does.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn a_gc7000_vertex_shader_with_up_to_sixteen_samplers_names_each_in_tex_id() {
        use crate::shaderc::online::vivante::decode::for_each_instr;
        let _serial = test_lock();
        for n in 9..=16usize {
            let mut vs = std::string::String::from("attribute vec4 a_pos;\nuniform vec4 u_w[4];\nvarying vec4 v_c;\n");
            for k in 0..n {
                vs += &std::format!("uniform sampler2D u_s{k};\n");
            }
            vs += "void main() {\n    vec2 c = vec2(0.5);\n    vec4 r = vec4(0.0);\n";
            for k in 0..n {
                vs += &std::format!(
                    "    r += u_w[{}].{} * texture2DLod(u_s{k}, c, 0.0);\n",
                    k / 4,
                    ['x', 'y', 'z', 'w'][k % 4]
                );
            }
            vs += "    v_c = r;\n    gl_Position = a_pos;\n}\n";
            let sh = compile_out(SHC_STAGE_VERTEX, &vs);
            // SAFETY: the record's code, `usc_dw` dwords, alive until the release below.
            let code = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
            let mut ids = std::vec::Vec::new();
            for_each_instr(code, |_, i| {
                if i.op().meta == "tex" {
                    ids.push(i.get("TEX_ID").expect("a tex op has TEX_ID"));
                }
                Ok(())
            })
            .expect("decodes");
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids, (0..n as u32).collect::<std::vec::Vec<_>>(), "{n} samplers: TEX_ID 0..{}", n - 1);
            let mut bindings: std::vec::Vec<u8> = sh.sampler_bindings().map(|(b, _)| b).collect();
            bindings.sort_unstable();
            assert_eq!(bindings, (0..n as u8).collect::<std::vec::Vec<_>>(), "{n} samplers: bindings");
            release(&sh);
        }
    }

    /// dEQP tmp_array.{vec2,vec3}_const_write_static_loop_read_vertex, as
    /// dEQP generates it: 40 elements, 36 of them literals written through
    /// `arr[i++]`, four read in a loop. Refused as
    /// `LoadBalancingTempsUnmeasured { temps: 45, temps_max: 21 }` while a
    /// masked literal seed stayed in a register
    /// (select.rs `a_vec2_or_vec3_literal_element_read_in_a_loop_folds_like_a_vec4_one`).
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn deqp_tmp_array_vec2_and_vec3_static_loop_reads_compile_on_the_vertex_stage() {
        let _serial = test_lock();
        for ty in ["vec2", "vec3"] {
            let k = if ty == "vec2" { "vec2(0.125, 0.25)" } else { "vec3(0.125, 0.25, 0.5)" };
            let out = if ty == "vec2" { "vec4(res, 0.0, 1.0)" } else { "vec4(res, 1.0)" };
            let mut vs = std::format!(
                "attribute highp vec4 a_position;\nattribute highp vec4 a_coords;\nvarying mediump vec4 v_color;\n\
                 void main()\n{{\n    gl_Position = a_position;\n    mediump {ty} arr[40];\n    arr[0] = {k};\n\
                 arr[1] = {k} * 0.5;\n    arr[2] = {k} * 0.25;\n    arr[3] = {k} * 0.125;\n    int i = 4;\n"
            );
            for n in 4..40 {
                vs += &std::format!("    arr[i++] = {ty}({n}.0);\n");
            }
            vs += &std::format!(
                "    mediump {ty} res = {ty}(0.0);\n    for (int i = 0; i < 4; i++)\n        res += arr[i];\n\
                 v_color = {out};\n}}\n\0"
            );
            let mut log = [0 as c_char; 256];
            let mut out = ShcShader::ZERO;
            // SAFETY: a NUL-terminated source, a real output, and a log of the
            // announced size.
            let rc = unsafe {
                online_compile(SHC_STAGE_VERTEX, vs.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len())
            };
            // SAFETY: the writer NUL-terminated the log.
            let text = String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }).into_owned();
            assert_eq!(rc, 0, "{ty}: {text}");
            release(&out);
        }
    }

    /// Never declared is the link error, and says so by that name.
    #[test]
    fn a_varying_the_vertex_shader_never_declares_is_not_declared() {
        let _serial = test_lock();
        let fs = fs_record(FS_READS_V, &["v"]);
        let (rc, log, fs) = link("attribute vec4 p; void main(){ gl_Position = p; }", fs);
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("'v' is not declared") && log.contains("4.3.5"), "{log}");
        release(&fs);
    }

    /// Declared with another type is an error whether or not it is written:
    /// dEQP `linkage.varying_type_mismatch_1` reads it only under
    /// `if (false)`.
    #[test]
    fn a_varying_of_another_type_is_refused_even_unwritten() {
        let _serial = test_lock();
        let fs = fs_record(FS_READS_V, &["v"]);
        let (rc, log, fs) = link("attribute vec4 p; varying vec2 v; void main(){ gl_Position = p; }", fs);
        assert_eq!(rc, -EINVAL);
        assert!(log.contains("another type") && log.contains("4.3.5"), "{log}");
        release(&fs);
    }

    /// Sections 4.3.4 and 4.5.3 for a struct uniform: its type is the whole
    /// declaration, so a member only one stage has, or a member precision
    /// that differs, refuses the link although no row the GL core sees
    /// differs. dEQP `linkage.uniform_struct_type_conflict_1` and
    /// `uniform_struct_precision_conflict_1..4` linked until 2026-09-23.
    /// The defaults count: the fragment side's `float` below is `mediump`
    /// by its `precision` statement.
    ///
    /// Checked: removing the `vu.ty != fu.ty` test makes every case link.
    #[test]
    fn a_struct_uniform_must_agree_member_for_member() {
        let _serial = test_lock();
        const VS: &str = "attribute vec4 p; struct S { lowp float a; }; uniform S s; \
                          void main(){ gl_Position = p * s.a; }";
        for (fs_struct, why) in [
            ("struct S { float a; };", "the default mediump against lowp"),
            ("struct S { highp float a; };", "highp against lowp"),
            ("struct S { lowp float a; lowp float b; };", "a member the vertex side lacks"),
            ("struct S { lowp int a; };", "int against float"),
        ] {
            let src = std::format!(
                "precision mediump float; {fs_struct} uniform S s; void main(){{ gl_FragColor = vec4(1.0); }}"
            );
            let (rc, log, fs) = link(VS, fs_record(&src, &[]));
            assert_eq!(rc, -EINVAL, "{why}: {log}");
            assert!(log.contains("'s'") && log.contains("4.5.3"), "{why}: {log}");
            release(&fs);
        }
        /* Written out, or left to agreeing defaults, it links. */
        for fs_struct in ["struct S { lowp float a; };", "precision lowp float; struct S { float a; };"] {
            let src = std::format!(
                "precision mediump float; {fs_struct} uniform S s; void main(){{ gl_FragColor = vec4(s.a); }}"
            );
            let (rc, log, fs) = link(VS, fs_record(&src, &[]));
            assert_eq!(rc, 0, "{log}");
            release(&fs);
        }
    }

    /// A plain uniform's precision is compared when both stages use it (the
    /// rule Mesa applies to ES 1.00): a vertex shader's default `highp`
    /// against a fragment `mediump` for a uniform one side ignores links.
    #[test]
    fn a_plain_uniform_precision_is_compared_when_both_stages_use_it() {
        let _serial = test_lock();
        let vs = "attribute vec4 p; uniform float k; void main(){ gl_Position = p * k; }";
        let used = "precision mediump float; uniform float k; void main(){ gl_FragColor = vec4(k); }";
        let (rc, log, fs) = link(vs, fs_record(used, &[]));
        assert_eq!(rc, -EINVAL, "{log}");
        assert!(log.contains("'k'") && log.contains("precision"), "{log}");
        release(&fs);
        let idle = "precision mediump float; uniform float k; void main(){ gl_FragColor = vec4(1.0); }";
        let (rc, log, fs) = link(vs, fs_record(idle, &[]));
        assert_eq!(rc, 0, "{log}");
        release(&fs);
        let agreed = "precision mediump float; uniform highp float k; void main(){ gl_FragColor = vec4(k); }";
        let (rc, log, fs) = link(vs, fs_record(agreed, &[]));
        assert_eq!(rc, 0, "{log}");
        release(&fs);
    }

    /* ---- the test lock covers every compile and every release ---------- */

    /// A compile's keep transaction is serialised with every other compile,
    /// whoever started it: a compile on another thread that holds its
    /// transaction open (the V3D backend's rig builds its clear program in
    /// v3d_init, and took no lock) makes this compile WAIT, not fail with
    /// "a compile was started inside a compile". The four v3d_abi tests
    /// failed together 4 runs in 20 for exactly this (2026-09-24).
    ///
    /// Revert check: without the lock in `begin_keep`, this compile finds
    /// the other thread's transaction open and fails at once.
    #[test]
    fn a_compile_waits_for_a_transaction_open_on_another_thread() {
        let (tx, rx) = std::sync::mpsc::channel();
        let other = std::thread::spawn(move || {
            /* no test_lock taken here, as the rig took none */
            let txn = store::begin_keep().expect("nothing else is open");
            tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            drop(txn);
        });
        rx.recv().unwrap();
        /* a source no other test compiles: a cache hit would not open keep */
        let (rc, log) = compile(SHC_STAGE_VERTEX, b"attribute vec4 p; void main(){ gl_Position = p * 1234.5; }");
        other.join().unwrap();
        assert_eq!(rc, 0, "{log}");
    }

    /// A release is serialised the same way: a test that holds the lock sees
    /// the keep region unchanged for as long as it holds it, whatever other
    /// threads release (the rig's v3d_shutdown releases its clear program).
    ///
    /// Revert check: without the lock in `release`, the region shrinks
    /// inside the other thread's window.
    #[test]
    fn a_release_waits_for_a_test_that_holds_the_lock() {
        let sh = compile_out(SHC_STAGE_VERTEX, "attribute vec4 p; void main(){ gl_Position = p * 4321.5; }");
        let (tx, rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let _s = test_lock();
            let before = store::keep_in_use();
            tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));
            (before, store::keep_in_use())
        });
        rx.recv().unwrap();
        release(&sh);
        let (before, after) = holder.join().unwrap();
        assert_eq!(before, after, "the region changed under a test that held the lock");
    }

    /// dEQP-GLES2 attribute_location.bind_aliasing.max_cond_float's vertex
    /// shader: 32 float attributes, `a_k` and `a_(k+16)` read under opposite
    /// conditions.
    fn aliasing_vs() -> String {
        let mut s = String::from("varying mediump vec4 v_color;\n");
        for k in 0..16 {
            s += &format!("attribute mediump float a_{k};\nattribute mediump float a_{};\n", k + 16);
        }
        s += "uniform mediump float u_A;\nvoid main (void)\n{\n\tmediump vec4 color = vec4(0.0);\n";
        for k in 0..16 {
            s += &format!("\tif (u_A != 0.0) color += vec4(a_{k});\n\tif (u_A == 0.0) color += vec4(a_{});\n", k + 16);
        }
        s += "\tv_color = color;\n\tgl_Position = color;\n}\n";
        s
    }

    /// Attributes that do not fit one location's registers each are left to
    /// the link: the compile succeeds with no code, `layout_at_link` and
    /// every declared attribute (no location yet), and `compile_vs_linked`
    /// with a map binding `a_k` and `a_(k+16)` to location k gives one row
    /// per location, 16, each with its own registers. With a map that binds
    /// all 32 apart, the relink is a refusal with the reason (it does not
    /// fit), not a second deferral. Every GPU: on the GC7000 a location is
    /// one vertex element and one input register; on the V3D one attribute
    /// record, and the stage record has no code (the program is built at
    /// link, from the source and the map the record keeps), so there the
    /// relinked record is linked with a fragment shader too: 16 records, one
    /// per location, each aliased pair fetched through its location's one.
    #[test]
    fn attributes_that_alias_only_at_link_compile_there() {
        use crate::shaderc::{ShcAttribBinding, ShcAttribMap, SHC_ATTRIB_NO_LOCATION};
        let src = aliasing_vs();
        let vs = compile_out(SHC_STAGE_VERTEX, &src);
        assert_eq!(vs.layout_at_link, 1, "deferred to the link");
        assert_eq!((vs.usc_dw, vs.num_attribs), (0, 0), "no code and no rows before the link");
        // SAFETY: the record's list of num_declared entries.
        let declared = unsafe { core::slice::from_raw_parts(vs.declared, vs.num_declared as usize) };
        let name = |b: &ShcAttribBinding| -> String {
            b.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
        };
        let names: Vec<String> = declared.iter().map(name).collect();
        let want: Vec<String> = (0..16).flat_map(|k| [format!("a_{k}"), format!("a_{}", k + 16)]).collect();
        assert_eq!(names, want, "every declared attribute, in declaration order");
        assert!(declared.iter().all(|d| d.location == SHC_ATTRIB_NO_LOCATION && d.columns == 1));
        assert!(declared.iter().all(|d| d.gl_type == 0x1406 && d.active == 1), "GL_FLOAT, and every one read");
        /* one never read: declared, inactive */
        let unread = src.replace("\tif (u_A == 0.0) color += vec4(a_31);\n", "");
        assert_ne!(unread, src, "the source changed");
        let vs2 = compile_out(SHC_STAGE_VERTEX, &unread);
        // SAFETY: the record's list of num_declared entries.
        let d2 = unsafe { core::slice::from_raw_parts(vs2.declared, vs2.num_declared as usize) };
        let inactive: Vec<String> = d2.iter().filter(|d| d.active == 0).map(name).collect();
        assert_eq!(inactive, ["a_31"], "exactly the attribute nothing loads");
        release(&vs2);

        let relink = |loc: &dyn Fn(usize) -> u8| -> (i32, ShcShader, String) {
            let mut map = ShcAttribMap::ZERO;
            map.count = declared.len() as u8;
            for (i, d) in declared.iter().enumerate() {
                let k: usize = name(d)[2..].parse().unwrap();
                map.b[i] = ShcAttribBinding { location: loc(k), ..*d };
            }
            let mut text = Vec::from(src.as_bytes());
            text.push(0);
            let mut log = [0 as c_char; 256];
            let mut out = ShcShader::ZERO;
            let entry = SHC_ONLINE.compile_vs_linked.expect("registered on every GPU");
            // SAFETY: a NUL-terminated source, a map, an output and a log of the announced size.
            let rc = unsafe { entry(text.as_ptr() as *const c_char, &map, &mut out, log.as_mut_ptr(), log.len()) };
            // SAFETY: the writer NUL-terminated the log.
            (rc, out, String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }).into_owned())
        };
        let (rc, out, log) = relink(&|k| (k % 16) as u8);
        assert_eq!(rc, 0, "{log}");
        assert_eq!(out.layout_at_link, 0);
        /* the V3D's stage record is reflection alone; its code is the program's */
        assert_eq!(out.usc_dw > 0, !cfg!(feature = "gpu-v3d"), "code at the link");
        let rows = &out.attribs[..out.num_attribs as usize];
        let mut locs: Vec<u8> = rows.iter().map(|r| r.location).collect();
        locs.sort_unstable();
        assert_eq!(locs, (0..16).collect::<Vec<u8>>(), "one row per location");
        let mut regs: Vec<u8> = rows.iter().map(|r| r.vi_reg).collect();
        regs.sort_unstable();
        regs.dedup();
        assert_eq!(regs.len(), 16, "each location its own registers: {regs:?}");
        #[cfg(feature = "gpu-v3d")]
        {
            use crate::shaderc::SHC_V3D_NO_RECORD;
            let mut fs = compile_out(
                SHC_STAGE_FRAGMENT,
                "varying mediump vec4 v_color;\nvoid main (void) { gl_FragColor = v_color; }",
            );
            let mut log = [0 as c_char; 512];
            // SAFETY: two distinct valid shaders and a log of the announced size.
            let rc = unsafe { online_link(&out, &mut fs, log.as_mut_ptr(), log.len()) };
            // SAFETY: the writer NUL-terminated the log.
            let text = String::from_utf8_lossy(unsafe { cstr_bytes(log.as_ptr()) }).into_owned();
            assert_eq!(rc, 0, "link: {text}");
            assert_ne!(fs.usc_dw, 0, "the link built the program");
            assert_eq!(fs.v3d.num_records, 16, "one record per location");
            let mut recs = fs.v3d.record_of_location.to_vec();
            assert!(!recs.contains(&SHC_V3D_NO_RECORD), "every location is read: {recs:?}");
            recs.sort_unstable();
            assert_eq!(recs, (0..16).collect::<Vec<u8>>(), "each location its own record");
            /* a float read: one VPM row per record */
            assert_eq!(fs.v3d.vs.vattr_sizes, [1; 16], "the aliased pair reads one row, the same");
            release(&fs);
        }
        release(&out);

        let (rc, _, log) = relink(&|k| k as u8);
        assert_ne!(rc, 0, "32 locations do not fit");
        /* the PowerVR's vertex-input register file; the GC7000's 16 rows;
         * the V3D's 16 attribute locations */
        let why = if cfg!(feature = "gpu-vivante") {
            "TooManyIo"
        } else if cfg!(feature = "gpu-v3d") {
            "AttribLocation { location: 16 }"
        } else {
            "VtxIn"
        };
        assert!(log.contains(why), "{log}");
        release(&vs);
    }
}
