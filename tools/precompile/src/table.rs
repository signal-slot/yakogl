// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The precompiled table: the records the compiler made, as Rust source.
//!
//! The idiom is `yakogl-tools`' emit.rs (the PowerVR table from Mesa): a row
//! is a real library struct, printed by destructuring it with a FULL pattern
//! (here [`print_row_and_reflection`], with the leaf printers shared through
//! shc_print.rs), so a field added to `ShcShader` fails this tool's build
//! until it says which half of the table the field belongs to. A row of ours
//! carries what a Mesa row cannot, as columns beside the rows, every one a
//! library type:
//!
//! | static | per row |
//! |---|---|
//! | `SHC_PRECOMPILED_TABLE: [ShcPrecompiledRow; N]` | the variant: its source, its source's reflection, the stage, the blend key, the code and the wiring a variant changes |
//! | `SHC_PRECOMPILED_KEYS: [[u8; 32]; N]` | the record key the compiler's cache finds it by: the SHA-256 chained with the zeroed varyings, the attribute map and the facts |
//! | `SHC_PRECOMPILED_W_CONSTANT: [bool; N]` | compiled for a program whose vertex shader writes a constant W |
//! | `SHC_PRECOMPILED_LOD_KEY: [u64; N]` | the sampler-state key (`LodKey::bits`) |
//! | `SHC_PRECOMPILED_IF_LOWERING: [u32; N]` | how an `if` with arithmetic arms was lowered (`IfLowering::as_u32`; the key covers it when not the default) |
//! | `SHC_PRECOMPILED_SENSITIVITY: [u64; N]` | the sampler-state sensitivity a program's first draw asks of it (`lod_sensitivity`, `LodKey::bits`) |
//! | `SHC_PRECOMPILED_ATTRIB_MAP: [Option<&ShcAttribMap>; N]` | the link's attribute locations a vertex shader was compiled with |
//!
//! A compile's reflection is stored once, in a `ShcPrecompiledReflection`
//! static (`S_<sha256>_<VS|FS>`, with `_DEFERRED` when the compile left its
//! attribute layout to the link) its rows point at: the varying and uniform
//! lists, the uniform blocks and the facts the compiler derived from the
//! source, which no variant of the compile changes. A compile is a source, its
//! stage, and whether its layout was left to the link. The whole list of every
//! source compiled at build time repeats 2,770,048 bytes of varyings and
//! attributes alone; storing them per compile instead of per row is what makes
//! the table cheaper than the compiler it stands in front of. The attribute
//! layout is per row, because a vertex shader that defers it is compiled twice:
//! once with no layout (no code, no attribute list, the sentinel offsets of a
//! record with none) and once with the map the link made, and the two records
//! differ exactly there. What keeps the split honest: [`emit`] refuses a compile
//! whose rows disagree on a field stored once (naming every field it sees), and
//! the library's tests compare every row and every recorded variant against what
//! the compiler makes of the same source with the table out of the way.
//!
//! Beside the rows: `SHC_PRECOMPILED_LINKS` (`ShcPrecompiledLink`), every
//! link the generator made by record key, the verdict a link takes without
//! the front end; and the toolchain mark as constants
//! (`SHC_PRECOMPILED_TOOLCHAIN`, `_TOOLCHAIN_C`, `_GPU`, `_ES2`), which the
//! library checks against its own GPU and ES level at build time, and the
//! list it was made from (`SHC_PRECOMPILED_LIST`, which a refusal names).
//!
//! Rows are sorted by source (list order), stage, blend key and record key,
//! and each row's and each compile's statics are named after their identity
//! (`R_<key>_<stage>_<blend key>`, `S_<sha256>_<stage>[_DEFERRED]`), never their
//! position, so a row added to the list renames nothing.

use crate::drive::hex;
use crate::shc_print::{
    binding_is_zero, emit_u32_array, print_array, print_binding, print_uniform, rs_comment, rs_str, ARRAY_ATTRIBS,
    ARRAY_VARYINGS, ATTRIBS, DECLARED, Field, UNIFORMS, VARYINGS,
};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use yakogl::shaderc::online::backend::RecordView;
use yakogl::shaderc::{
    ShcAttribBinding, ShcAttribMap, ShcBlock, ShcBlockMember, ShcShader, ShcUniform, SHC_STAGE_FRAGMENT,
};

/// Where a row came from: the job, and the step of it that made the record
/// first. What `--verify` replays, alone, to make the row again.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// A source compiled on its own (no listed program links it), and for a
    /// fragment shader its variants of the plain record.
    Alone { src: String },
    /// A listed program: its two sources compiled, linked, and the variants
    /// asked of the linked fragment shader.
    Pair { vs: String, fs: String },
}

/// The step of the job that made a row: the compiles, the link, or one
/// variant (and its link).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    Compile,
    Link,
    Variant(u32),
}

/// One row: a record, copied out of the compiler's keep region (which the
/// job gives back) into memory this process owns.
pub struct Row {
    pub key: [u8; 32],
    pub sha: [u8; 32],
    pub source: usize,
    pub stage: u32,
    pub blend_key: u32,
    pub w_constant: bool,
    pub lod_key: u64,
    /// `IfLowering::as_u32` of the record.
    pub if_lowering: u32,
    /// `lod_sensitivity` of the record (set by the caller, which asks it of
    /// the live record).
    pub sensitivity: u64,
    pub map: Option<Box<ShcAttribMap>>,
    /// The shader, its pointers at the vectors below.
    pub shader: ShcShader,
    pub blocks: Vec<ShcBlock>,
    pub members: Vec<ShcBlockMember>,
    pub arrays: Vec<u16>,
    pub origin: (Origin, Step),
    code: Vec<u32>,
    uniforms: Vec<ShcUniform>,
    declared: Vec<ShcAttribBinding>,
    #[cfg(feature = "gpu-vivante")]
    const_contents: Vec<u8>,
    #[cfg(feature = "gpu-vivante")]
    const_data: Vec<u32>,
}

/// A slice from a pointer and a count that may be null and zero.
///
/// # Safety
/// A non-null `p` points at `n` live `T`s.
unsafe fn slice<'a, T>(p: *const T, n: usize) -> &'a [T] {
    if p.is_null() || n == 0 {
        &[]
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(p, n) }
    }
}

/// A name copied out of the keep region, for as long as this process runs.
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

impl Row {
    /// The record `r`, copied while its record is live.
    pub fn of(r: &RecordView, source: usize, origin: (Origin, Step)) -> Row {
        let mut sh = r.shader;
        // SAFETY: the record is live (the caller holds it) and its shader's
        // pointers and counts describe its keep extent.
        let (code, uniforms, declared) = unsafe {
            (
                slice(sh.usc, sh.usc_dw as usize).to_vec(),
                slice(sh.uniforms, sh.num_uniforms as usize).to_vec(),
                slice(sh.declared, sh.num_declared as usize).to_vec(),
            )
        };
        #[cfg(feature = "gpu-vivante")]
        let (const_contents, const_data) = (sh.viv.const_contents().to_vec(), sh.viv.const_data().to_vec());
        let blocks: Vec<ShcBlock> = r.blocks.blocks.iter().map(|b| ShcBlock { name: leak(b.name), ..*b }).collect();
        let members: Vec<ShcBlockMember> =
            r.blocks.members.iter().map(|m| ShcBlockMember { name: leak(m.name), ..*m }).collect();
        /* the row owns everything its shader points at; it is not a record */
        sh.priv_ = core::ptr::null();
        let mut row = Row {
            key: r.key,
            sha: yakogl::shaderc::sha256::sha256(r.source),
            source,
            stage: r.stage,
            blend_key: r.blend_key,
            w_constant: r.w_constant,
            lod_key: r.lod_key.bits(),
            if_lowering: r.if_lowering.as_u32(),
            sensitivity: 0,
            map: r.attrib_map.map(|m| Box::new(*m)),
            shader: sh,
            blocks,
            members,
            arrays: r.blocks.arrays.to_vec(),
            origin,
            code,
            uniforms,
            declared,
            #[cfg(feature = "gpu-vivante")]
            const_contents,
            #[cfg(feature = "gpu-vivante")]
            const_data,
        };
        row.repoint();
        row
    }

    /// Point the shader at this row's own vectors.
    fn repoint(&mut self) {
        fn ptr<T>(v: &[T]) -> *const T {
            if v.is_empty() {
                core::ptr::null()
            } else {
                v.as_ptr()
            }
        }
        self.shader.usc = ptr(&self.code);
        self.shader.uniforms = ptr(&self.uniforms);
        self.shader.declared = ptr(&self.declared);
        #[cfg(feature = "gpu-vivante")]
        {
            self.shader.viv.const_contents = ptr(&self.const_contents);
            self.shader.viv.const_data = ptr(&self.const_data);
        }
    }

    /// The row's identity: what the cache finds it by.
    pub fn id(&self) -> crate::drive::Id {
        (self.key, self.stage, self.blend_key)
    }

    /// The name its statics are given: its identity, never its position.
    pub fn ident(&self) -> String {
        let st = if self.stage == SHC_STAGE_FRAGMENT { "FS" } else { "VS" };
        format!("R_{}_{st}_{:08X}", hex(&self.key[..8]).to_uppercase(), self.blend_key)
    }

    /// Instruction bytes: what the row costs an image in code.
    pub fn code_bytes(&self) -> usize {
        self.code.len() * 4
    }
}

/// A source of the table: its name (the path under tools/shaders/) and text.
pub struct Source {
    pub name: String,
    pub text: String,
}

/// What [`emit`] needs besides the rows.
pub struct Header<'a> {
    /// `// toolchain: ...`, the line after the mark.
    pub toolchain: &'a str,
    /// The source list the table was made from, as a refusal names it.
    pub list: &'a str,
    /// The GPU and ES level the mark names, as constants the library checks.
    pub gpu: &'a str,
    pub es2: bool,
    /// What the table was made from, one comment line each.
    pub about: &'a [String],
}

/// The fixed-array printers of the GC7000's register lists.
#[cfg(feature = "gpu-vivante")]
mod viv {
    use crate::shc_print::{name_str, ArrayField, Field};
    use yakogl::shaderc::ShcVivIo;

    pub fn print_io(e: &ShcVivIo) -> String {
        let ShcVivIo { reg, slot, components, interp, name } = *e;
        format!(
            "ShcVivIo {{ reg: {reg}, slot: {slot}, components: {components}, interp: {interp}, name: cname(\"{}\") }}",
            name_str(&name)
        )
    }

    pub fn io_is_zero(e: &ShcVivIo) -> bool {
        let ShcVivIo { reg, slot, components, interp, name } = *e;
        reg == 0 && slot == 0 && components == 0 && interp == 0 && name.iter().all(|&c| c == 0)
    }

    pub const IN: ArrayField<ShcVivIo> = ArrayField {
        kind: Field { field: "in_", ty: "ShcVivIo", is_zero: io_is_zero, print: print_io },
        pad: "pad_io",
        zero: "ShcVivIo::ZERO",
        len: "SHC_VIV_MAX_IO",
    };
    pub const OUT: ArrayField<ShcVivIo> = ArrayField { kind: Field { field: "out", ..IN.kind }, ..IN };
}

/// The name a source's reflection statics are given: the source's own
/// SHA-256, the stage and whether the compile left its attribute layout to
/// the link, never its position. The last part is what separates a source's
/// two compiles, which are not variants of each other ([`emit`]).
pub fn reflection_ident(r: &Row) -> String {
    let st = if r.stage == SHC_STAGE_FRAGMENT { "FS" } else { "VS" };
    let deferred = if r.shader.layout_at_link != 0 { "_DEFERRED" } else { "" };
    format!("S_{}_{st}{deferred}", hex(&r.sha[..8]).to_uppercase())
}

/// One field of a row or of a source's reflection: the static its entries are
/// stored in (empty when the field has none) and the line the struct stores.
/// Keeping a field's two parts together is what lets [`emit`] compare the rows
/// of one source field by field and name the fields they disagree on -- two
/// rows can point at the same static name and fill it differently.
#[derive(PartialEq, Eq)]
struct ReflField {
    name: &'static str,
    statics: Vec<String>,
    entry: String,
}

/// A field that is a number, not a list: no static, one line.
fn fact(name: &'static str, entry: String) -> ReflField {
    ReflField { name, statics: Vec::new(), entry }
}

/// The fields two rows of one compile disagree on, named. Compared field by
/// field, and a field's stored entries with its line: two rows can point at
/// the same static name and fill it differently, and the reader of the refusal
/// needs the names, not the first difference.
fn disagreements(had: &[ReflField], new: &[ReflField]) -> Vec<String> {
    had
        .iter()
        .zip(new)
        .filter(|(x, y)| x != y)
        .map(|(x, y)| {
            let stored = if x.statics == y.statics { "" } else { ", and their stored entries differ too" };
            format!(
                "    {}: `{}` in one row, `{}` in the other{stored}",
                x.name,
                x.entry.trim_end_matches(','),
                y.entry.trim_end_matches(',')
            )
        })
        .collect()
}

/// `static {name}: [ty; n] = [...]` of the entries `f` describes (no padding:
/// a [`ShcShader`] pads them at the look-up), and the `field: &{name},` line
/// that points at it -- `field: &[],` when there are none.
fn print_list<T>(name: &str, f: &Field<T>, items: &[T], used: usize) -> Result<ReflField, String> {
    let Field { field, ty, is_zero, print } = f;
    if items[used.min(items.len())..].iter().any(|x| !is_zero(x)) {
        return Err(format!("{field}: non-zero entries past the count"));
    }
    if used == 0 {
        return Ok(ReflField { name: field, statics: Vec::new(), entry: format!("{field}: &[],") });
    }
    let mut statics = vec![format!("static {name}: [{ty}; {used}] = [")];
    for x in &items[..used] {
        statics.push(format!("    {},", print(x)));
    }
    statics.push("];".into());
    Ok(ReflField { name: field, statics, entry: format!("{field}: &{name},") })
}

/// What the table makes of one row: its `ShcPrecompiledRow` entry, its
/// source's reflection as one field per entry, and the statics the row's own
/// list fields point at. They are separate because they are stored on
/// different terms: a source's fields are written once for all of its rows, a
/// row's are written for that row.
struct Emitted {
    row: Vec<String>,
    /// What the source's reflection static is named (`S_<sha256>_<VS|FS>`).
    rid: String,
    /// The reflection's fields, in the order the struct declares them.
    fields: Vec<ReflField>,
    row_lists: Vec<String>,
}

/// The source's reflection as the table stores it: the statics its list fields
/// point at, then the static that holds the fields.
fn reflection_static(e: &Emitted) -> Vec<String> {
    let mut out = Vec::new();
    for f in &e.fields {
        out.extend(f.statics.iter().cloned());
    }
    out.push(format!("static {}: ShcPrecompiledReflection = ShcPrecompiledReflection {{", e.rid));
    for f in &e.fields {
        out.push(format!("    {}", f.entry));
    }
    out.push("};".into());
    out
}

/// Row `r` as the table's two halves: its `ShcPrecompiledRow` entry and its
/// source's `ShcPrecompiledReflection` static with the statics it points at.
///
/// Every field of `ShcShader` is named here and put in one of the two, so a
/// field added to the library fails this tool's build until it is classified
/// (the idiom [`crate::shc_print`] describes). The row gets what a variant of
/// the source changes -- its code, the wiring the blend state decides, and the
/// attribute layout a link decides -- and the reflection gets what the source
/// itself says, which no variant of it changes.
fn print_row_and_reflection(r: &Row, sources: &[Source]) -> Result<Emitted, String> {
    let (id, rid) = (r.ident(), reflection_ident(r));
    let ShcShader {
        usc,
        usc_dw,
        temps,
        varyings,
        num_varyings,
        uses_w,
        uses_discard,
        reads_front_facing,
        meta_off,
        fs_front_face_off1,
        num_outputs,
        num_tile_buffers,
        tile_buffer_sh,
        attribs,
        num_attribs,
        vs_output_size,
        vs_psize_off,
        vs_vertex_id_reg1,
        vs_instance_id_reg1,
        uniform_dwords,
        uniform_sh_dwords,
        uniforms,
        num_uniforms,
        /* what the compiler wrote in the shader it made, which is not always
         * the record's own key (`r.blend_key`): see [`ShcPrecompiledRow`] */
        blend_key: shader_blend_key,
        uses_fbfetch,
        blend_consts_off,
        uses_pntc,
        uses_z,
        fragcoord_ytf_off1,
        native_cube,
        layout_at_link,
        declared,
        num_declared,
        #[cfg(feature = "gpu-vivante")]
        viv,
        priv_,
    } = r.shader;
    if !priv_.is_null()
        || usc.is_null() != (usc_dw == 0)
        || uniforms.is_null() != (num_uniforms == 0)
        || declared.is_null() != (num_declared == 0)
        /* a stored count must fit the fixed array [`materialize`](yakogl::shaderc::precompiled::materialize)
         * pads it into: a row or a reflection keeps `count` entries, no padding */
        || num_varyings as usize > varyings.len()
        || num_attribs as usize > attribs.len()
    {
        return Err(format!("{id}: pointers and counts disagree"));
    }
    /* the row's source index must name a source of this table */
    sources.get(r.source).ok_or_else(|| format!("{id}: source {} out of range", r.source))?;

    /* ---- the row: the variant ---- */
    let mut row = Vec::new();
    let mut row_lists = Vec::new();
    let f = |row: &mut Vec<String>, text: String| row.push(format!("    {text}"));
    f(&mut row, "ShcPrecompiledRow {".into());
    let g = |row: &mut Vec<String>, text: String| row.push(format!("        {text}"));
    g(&mut row, format!("source: {},", r.source));
    g(&mut row, format!("reflection: &{rid},"));
    g(
        &mut row,
        format!("stage: {},", if r.stage == SHC_STAGE_FRAGMENT { "SHC_STAGE_FRAGMENT" } else { "SHC_STAGE_VERTEX" }),
    );
    g(&mut row, format!("blend_key: 0x{:x},", r.blend_key));
    g(&mut row, format!("shader_blend_key: 0x{shader_blend_key:x},"));
    g(&mut row, format!("usc: {},", if usc.is_null() { "&[]".to_string() } else { format!("&{id}_USC") }));
    g(&mut row, format!("temps: {temps},"));
    g(&mut row, format!("num_outputs: {num_outputs},"));
    g(&mut row, format!("uniform_dwords: {uniform_dwords},"));
    g(&mut row, format!("uses_fbfetch: {uses_fbfetch},"));
    g(&mut row, format!("blend_consts_off: 0x{blend_consts_off:x},"));
    /* the attribute layout this record was compiled with, and whether the
     * compile left it to the link: the row's own, because the GC7000 makes two
     * records of one source that differ here, one with `attribs` and one with
     * `declared` (ShcPrecompiledRow::attribs) */
    for l in [
        print_list(&format!("{id}_ATTRIBS"), &ATTRIBS, &attribs, num_attribs as usize)?,
        print_list(&format!("{id}_DECLARED"), &DECLARED, &r.declared, num_declared as usize)?,
    ] {
        row_lists.extend(l.statics);
        g(&mut row, l.entry);
    }
    g(&mut row, format!("layout_at_link: {layout_at_link},"));
    #[cfg(feature = "gpu-vivante")]
    print_viv(&mut row, &viv, &id, "        ")?;
    g(&mut row, "},".into());

    /* ---- the reflection: the source, stored once ---- */
    let mut fields = Vec::new();
    fields.push(print_list(&format!("{rid}_VARYINGS"), &VARYINGS, &varyings, num_varyings as usize)?);
    fields.push(print_list(&format!("{rid}_UNIFORMS"), &UNIFORMS, &r.uniforms, num_uniforms as usize)?);
    for (name, entry) in [
        ("uses_w", format!("uses_w: {uses_w},")),
        ("uses_discard", format!("uses_discard: {uses_discard},")),
        ("reads_front_facing", format!("reads_front_facing: {reads_front_facing},")),
        ("meta_off", format!("meta_off: 0x{meta_off:x},")),
        ("fs_front_face_off1", format!("fs_front_face_off1: {fs_front_face_off1},")),
        ("num_tile_buffers", format!("num_tile_buffers: {num_tile_buffers},")),
        ("tile_buffer_sh", format!("tile_buffer_sh: {tile_buffer_sh},")),
        ("vs_output_size", format!("vs_output_size: {vs_output_size},")),
        ("vs_psize_off", format!("vs_psize_off: 0x{vs_psize_off:x},")),
        ("vs_vertex_id_reg1", format!("vs_vertex_id_reg1: {vs_vertex_id_reg1},")),
        ("vs_instance_id_reg1", format!("vs_instance_id_reg1: {vs_instance_id_reg1},")),
        ("uniform_sh_dwords", format!("uniform_sh_dwords: {uniform_sh_dwords},")),
        ("uses_pntc", format!("uses_pntc: {uses_pntc},")),
        ("uses_z", format!("uses_z: {uses_z},")),
        ("fragcoord_ytf_off1", format!("fragcoord_ytf_off1: {fragcoord_ytf_off1},")),
        ("native_cube", format!("native_cube: {native_cube},")),
        ("blocks", format!("blocks: {},", blocks_text(r))),
    ] {
        fields.push(fact(name, entry));
    }
    Ok(Emitted { row, rid, fields, row_lists })
}

/// `ShcShader { ... }` of row `ident`, every field named, its lines indented
/// by `ind`. The `--internal` file's row keeps the whole reflection inside one
/// shader ([`row_text`]); a precompiled table splits it into a row and a
/// per-source reflection ([`print_row_and_reflection`]).
fn print_shader(out: &mut Vec<String>, s: &ShcShader, ident: &str, ind: &str) -> Result<(), String> {
    let ShcShader {
        usc,
        usc_dw,
        temps,
        varyings,
        num_varyings,
        uses_w,
        uses_discard,
        reads_front_facing,
        meta_off,
        fs_front_face_off1,
        num_outputs,
        num_tile_buffers,
        tile_buffer_sh,
        attribs,
        num_attribs,
        vs_output_size,
        vs_psize_off,
        vs_vertex_id_reg1,
        vs_instance_id_reg1,
        uniform_dwords,
        uniform_sh_dwords,
        uniforms,
        num_uniforms,
        blend_key,
        uses_fbfetch,
        blend_consts_off,
        uses_pntc,
        uses_z,
        fragcoord_ytf_off1,
        native_cube,
        layout_at_link,
        declared,
        num_declared,
        #[cfg(feature = "gpu-vivante")]
        viv,
        priv_,
    } = *s;
    if !priv_.is_null()
        || usc.is_null() != (usc_dw == 0)
        || uniforms.is_null() != (num_uniforms == 0)
        || declared.is_null() != (num_declared == 0)
    {
        return Err(format!("{ident}: pointers and counts disagree"));
    }
    let f = |out: &mut Vec<String>, text: String| out.push(format!("{ind}{text}"));
    let ptr = |p: bool, what: &str| if p { format!("{ident}_{what}.as_ptr()") } else { "core::ptr::null()".into() };
    f(out, format!("usc: {},", ptr(!usc.is_null(), "USC")));
    f(out, format!("usc_dw: {usc_dw},"));
    f(out, format!("temps: {temps},"));
    print_array(out, ind, &ARRAY_VARYINGS, &varyings, num_varyings as usize).map_err(|e| format!("{ident}: {e}"))?;
    f(out, format!("num_varyings: {num_varyings},"));
    f(out, format!("uses_w: {uses_w},"));
    f(out, format!("uses_discard: {uses_discard},"));
    f(out, format!("reads_front_facing: {reads_front_facing},"));
    f(out, format!("meta_off: 0x{meta_off:x},"));
    f(out, format!("fs_front_face_off1: {fs_front_face_off1},"));
    f(out, format!("num_outputs: {num_outputs},"));
    f(out, format!("num_tile_buffers: {num_tile_buffers},"));
    f(out, format!("tile_buffer_sh: {tile_buffer_sh},"));
    print_array(out, ind, &ARRAY_ATTRIBS, &attribs, num_attribs as usize).map_err(|e| format!("{ident}: {e}"))?;
    f(out, format!("num_attribs: {num_attribs},"));
    f(out, format!("vs_output_size: {vs_output_size},"));
    f(out, format!("vs_psize_off: 0x{vs_psize_off:x},"));
    f(out, format!("vs_vertex_id_reg1: {vs_vertex_id_reg1},"));
    f(out, format!("vs_instance_id_reg1: {vs_instance_id_reg1},"));
    f(out, format!("uniform_dwords: {uniform_dwords},"));
    f(out, format!("uniform_sh_dwords: {uniform_sh_dwords},"));
    f(out, format!("uniforms: {},", ptr(!uniforms.is_null(), "UNIFORMS")));
    f(out, format!("num_uniforms: {num_uniforms},"));
    f(out, format!("blend_key: 0x{blend_key:x},"));
    f(out, format!("uses_fbfetch: {uses_fbfetch},"));
    f(out, format!("blend_consts_off: 0x{blend_consts_off:x},"));
    f(out, format!("uses_pntc: {uses_pntc},"));
    f(out, format!("uses_z: {uses_z},"));
    f(out, format!("fragcoord_ytf_off1: {fragcoord_ytf_off1},"));
    f(out, format!("native_cube: {native_cube},"));
    f(out, format!("layout_at_link: {layout_at_link},"));
    f(out, format!("declared: {},", ptr(!declared.is_null(), "DECLARED")));
    f(out, format!("num_declared: {num_declared},"));
    #[cfg(feature = "gpu-vivante")]
    print_viv(out, &viv, ident, ind)?;
    f(out, "priv_: core::ptr::null(),".into());
    Ok(())
}

/// The statics row `r` points at, and its `ShcBlob` entry: the `--internal`
/// file's row, which keeps the whole reflection in its shader (the columns a
/// precompiled table has are in [`emit`]).
pub fn row_text(r: &Row, sources: &[Source]) -> Result<(Vec<String>, Vec<String>), String> {
    let id = r.ident();
    let mut st = Vec::new();
    if !r.code.is_empty() {
        emit_u32_array(&mut st, &format!("{id}_USC"), &r.code);
    }
    if !r.uniforms.is_empty() {
        st.push(format!("static {id}_UNIFORMS: [ShcUniform; {}] = [", r.uniforms.len()));
        st.extend(r.uniforms.iter().map(|u| format!("    {},", print_uniform(u))));
        st.push("];".into());
    }
    if !r.declared.is_empty() {
        st.push(format!("static {id}_DECLARED: [ShcAttribBinding; {}] = [", r.declared.len()));
        st.extend(r.declared.iter().map(|b| format!("    {},", print_binding(b))));
        st.push("];".into());
    }
    #[cfg(feature = "gpu-vivante")]
    if !r.const_contents.is_empty() {
        st.push(format!("static {id}_CONST_CONTENTS: [u8; {}] = {:?};", r.const_contents.len(), r.const_contents));
        emit_u32_array(&mut st, &format!("{id}_CONST_DATA"), &r.const_data);
    }
    if let Some(m) = &r.map {
        st.push(format!("static {id}_ATTRIB_MAP: ShcAttribMap = {};", print_map(m, &id)?));
    }
    let mut row = vec!["    ShcBlob {".to_string()];
    row.push(format!(
        "        sha256: [{}],",
        r.sha.iter().map(|b| format!("0x{b:02x}")).collect::<Vec<_>>().join(", ")
    ));
    let name = &sources.get(r.source).ok_or_else(|| format!("{id}: source {} out of range", r.source))?.name;
    /* NUL-terminated, as blobs.rs: shc_shader_source_name() hands C the bytes */
    row.push(format!("        name: \"{}\\0\",", rs_str(name)));
    row.push(format!(
        "        stage: {},",
        if r.stage == SHC_STAGE_FRAGMENT { "SHC_STAGE_FRAGMENT" } else { "SHC_STAGE_VERTEX" }
    ));
    row.push(format!("        blend_key: 0x{:x},", r.blend_key));
    row.push("        shader: ShcShader {".into());
    print_shader(&mut row, &r.shader, &id, "            ")?;
    row.push("        },".into());
    row.push("    },".into());
    Ok((st, row))
}

/// The statics row `id` points at that are not a list: its code, its attribute
/// map and (GC7000) its constant register file. The lists -- a source's, and
/// the row's own attribute layout -- are in [`print_row_and_reflection`].
fn row_statics(r: &Row) -> Result<Vec<String>, String> {
    let id = r.ident();
    let mut st = Vec::new();
    if !r.code.is_empty() {
        emit_u32_array(&mut st, &format!("{id}_USC"), &r.code);
    }
    #[cfg(feature = "gpu-vivante")]
    if !r.const_contents.is_empty() {
        st.push(format!("static {id}_CONST_CONTENTS: [u8; {}] = {:?};", r.const_contents.len(), r.const_contents));
        emit_u32_array(&mut st, &format!("{id}_CONST_DATA"), &r.const_data);
    }
    if let Some(m) = &r.map {
        st.push(format!("static {id}_ATTRIB_MAP: ShcAttribMap = {};", print_map(m, &id)?));
    }
    Ok(st)
}

/// `viv: ShcViv { ... },`, every field named.
#[cfg(feature = "gpu-vivante")]
fn print_viv(out: &mut Vec<String>, v: &yakogl::shaderc::ShcViv, ident: &str, ind: &str) -> Result<(), String> {
    let yakogl::shaderc::ShcViv {
        in_,
        num_in,
        out: outs,
        num_out,
        vs_reg_of_in,
        vs_pos_out_reg,
        vs_pointsize_out_reg,
        vs_id_in_reg,
        ps_color_out_reg,
        ps_depth_out_reg,
        vs_load_balancing,
        input_count_unk8,
        const_contents,
        const_data,
        const_count,
    } = *v;
    if const_contents.is_null() != (const_count == 0) || const_data.is_null() != (const_count == 0) {
        return Err(format!("{ident}: the constant file's pointers and count disagree"));
    }
    let vi = format!("{ind}    ");
    let f = |out: &mut Vec<String>, text: String| out.push(format!("{vi}{text}"));
    let ptr = |p: bool, what: &str| if p { format!("{ident}_{what}.as_ptr()") } else { "core::ptr::null()".into() };
    out.push(format!("{ind}viv: ShcViv {{"));
    print_array(out, &vi, &viv::IN, &in_, num_in as usize).map_err(|e| format!("{ident}: {e}"))?;
    f(out, format!("num_in: {num_in},"));
    print_array(out, &vi, &viv::OUT, &outs, num_out as usize).map_err(|e| format!("{ident}: {e}"))?;
    f(out, format!("num_out: {num_out},"));
    f(out, format!("vs_reg_of_in: {:?},", vs_reg_of_in));
    f(out, format!("vs_pos_out_reg: {vs_pos_out_reg},"));
    f(out, format!("vs_pointsize_out_reg: {vs_pointsize_out_reg},"));
    f(out, format!("vs_id_in_reg: {vs_id_in_reg},"));
    f(out, format!("ps_color_out_reg: {ps_color_out_reg},"));
    f(out, format!("ps_depth_out_reg: {ps_depth_out_reg},"));
    f(out, format!("vs_load_balancing: 0x{vs_load_balancing:08x},"));
    f(out, format!("input_count_unk8: {input_count_unk8},"));
    f(out, format!("const_contents: {},", ptr(!const_contents.is_null(), "CONST_CONTENTS")));
    f(out, format!("const_data: {},", ptr(!const_data.is_null(), "CONST_DATA")));
    f(out, format!("const_count: {const_count},"));
    out.push(format!("{ind}}},"));
    Ok(())
}

fn print_block(b: &ShcBlock) -> String {
    let ShcBlock { name, data_size, first_member, num_members } = *b;
    format!(
        "ShcBlock {{ name: \"{}\", data_size: {data_size}, first_member: {first_member}, num_members: {num_members} }}",
        rs_str(name)
    )
}

fn print_member(m: &ShcBlockMember) -> String {
    let ShcBlockMember { name, row, offset, array_stride, matrix_stride, row_major, gl_type, array_size } = *m;
    format!(
        "ShcBlockMember {{ name: \"{}\", row: {row}, offset: {offset}, array_stride: {array_stride}, matrix_stride: {matrix_stride}, row_major: {row_major}, gl_type: 0x{gl_type:04x}, array_size: {array_size} }}",
        rs_str(name)
    )
}

/// `ShcAttribMap { ... }`, all of its fixed array (unused entries `ZERO`).
fn print_map(m: &ShcAttribMap, ident: &str) -> Result<String, String> {
    let ShcAttribMap { count, b } = m;
    if b[(*count as usize).min(b.len())..].iter().any(|x| !binding_is_zero(x)) {
        return Err(format!("{ident}: attribute map entries past its count"));
    }
    let mut s = format!("ShcAttribMap {{\n    count: {count},\n    b: [\n");
    for x in b.iter() {
        if binding_is_zero(x) {
            s.push_str("        ShcAttribBinding::ZERO,\n");
        } else {
            let _ = writeln!(s, "        {},", print_binding(x));
        }
    }
    s.push_str("    ],\n}");
    Ok(s)
}

/// `ShcBlocks { ... }` of row `r`: its uniform blocks, inline.
fn blocks_text(r: &Row) -> String {
    if r.blocks.is_empty() && r.members.is_empty() && r.arrays.is_empty() {
        return "ShcBlocks::EMPTY".to_string();
    }
    let list = |v: Vec<String>| if v.is_empty() { "&[]".to_string() } else { format!("&[{}]", v.join(", ")) };
    format!(
        "ShcBlocks {{ blocks: {}, members: {}, arrays: {} }}",
        list(r.blocks.iter().map(print_block).collect()),
        list(r.members.iter().map(print_member).collect()),
        list(r.arrays.iter().map(|a| a.to_string()).collect()),
    )
}

/// Each column's entry for row `r`, in [`COLUMNS`] order.
fn column_entries(r: &Row) -> [String; COLUMNS.len()] {
    let id = r.ident();
    [
        format!("[{}]", r.key.iter().map(|b| format!("0x{b:02x}")).collect::<Vec<_>>().join(", ")),
        r.w_constant.to_string(),
        format!("0x{:016x}", r.lod_key),
        r.if_lowering.to_string(),
        format!("0x{:016x}", r.sensitivity),
        if r.map.is_some() { format!("Some(&{id}_ATTRIB_MAP)") } else { "None".into() },
    ]
}

/// The column statics: name, element type, what one entry is. Every one of
/// them is a fact of the variant -- an input of the record key, or the answer
/// a program's first draw asks -- which is why they stay per row while the
/// source's reflection does not.
const COLUMNS: [(&str, &str, &str); 6] = [
    (
        "SHC_PRECOMPILED_KEYS",
        "[u8; 32]",
        "The record key the compiler's cache finds the row by: the SHA-256 of the\n/// source chained with the zeroed varyings, the attribute map and the facts.",
    ),
    (
        "SHC_PRECOMPILED_W_CONSTANT",
        "bool",
        "Compiled for a program whose vertex shader writes a constant `gl_Position.w`.",
    ),
    ("SHC_PRECOMPILED_LOD_KEY", "u64", "The sampler-state key it was compiled for (`LodKey::bits`; 0: none)."),
    (
        "SHC_PRECOMPILED_IF_LOWERING",
        "u32",
        "How an `if` with arithmetic arms was lowered (`IfLowering::as_u32`, `PVR_IF_LOWERING_*`;\n/// 0: the default, which the key does not chain).",
    ),
    (
        "SHC_PRECOMPILED_SENSITIVITY",
        "u64",
        "The sampler-state sensitivity a program's first draw asks of the row (`LodKey::bits`).",
    ),
    (
        "SHC_PRECOMPILED_ATTRIB_MAP",
        "Option<&ShcAttribMap>",
        "The link's attribute locations a vertex shader was compiled with.",
    ),
];

/// One row as text, all of it: its statics, its source's reflection (statics
/// and entry), its own statics, its table entry and its column entries. Two
/// rows are the same row exactly when these are equal -- which is what
/// `--verify` compares a freshly compiled row against, so a row is proven to
/// be the whole record the compiler makes for it, split or not.
pub fn fingerprint(r: &Row, sources: &[Source]) -> Result<String, String> {
    let p = print_row_and_reflection(r, sources)?;
    Ok(format!(
        "{}\n{}\n{}\n{}\n{}",
        row_statics(r)?.join("\n"),
        reflection_static(&p).join("\n"),
        p.row_lists.join("\n"),
        p.row.join("\n"),
        column_entries(r).join("\n")
    ))
}

/// The whole table as Rust source.
pub fn emit(
    rows: &[Row],
    sources: &[Source],
    links: &[yakogl::shaderc::ShcPrecompiledLink],
    h: &Header<'_>,
) -> Result<String, String> {
    let mut out: Vec<String> = vec![
        "// SPDX-License-Identifier: Apache-2.0".into(),
        "//".into(),
        "// GENERATED by `yakogl-precompile` (tools/precompile) from tools/shaders/precompiled.list.".into(),
        "// DO NOT EDIT, and do not commit: an image build writes it into build/.".into(),
        "//".into(),
        "// Every record YakoGL's own shader compiler (src/shaderc/online) made when the list's".into(),
        "// sources were compiled, the listed programs linked and their variants asked for, on the".into(),
        "// host, through the product back end's own entry points in the order the GL core calls".into(),
        "// them. The toolchain is this repository's compiler at the commit named below.".into(),
    ];
    out.extend(h.about.iter().map(|l| rs_comment(&format!("// {l}"))));
    out.push("//".into());
    out.push(format!("// toolchain: {}", h.toolchain));
    out.push("//".into());
    out.push(format!(
        "// {} rows of {} sources; row, stage, blend key, facts, and the step that made it:",
        rows.len(),
        sources.len()
    ));
    for r in rows {
        let mut facts = String::new();
        if r.w_constant {
            facts.push_str(" w-constant");
        }
        if r.lod_key != 0 {
            let _ = write!(facts, " lod=0x{:x}", r.lod_key);
        }
        if r.if_lowering != 0 {
            let _ = write!(facts, " if-lowering={}", r.if_lowering);
        }
        if r.map.is_some() {
            facts.push_str(" attrib-map");
        }
        let (o, step) = &r.origin;
        let from = match o {
            Origin::Alone { .. } => "alone".to_string(),
            Origin::Pair { vs, fs } => format!("pair {vs} {fs}"),
        };
        let step = match step {
            Step::Compile => "compile".to_string(),
            Step::Link => "link".to_string(),
            Step::Variant(k) => format!("variant 0x{k:x}"),
        };
        out.push(rs_comment(&format!(
            "//   {} {} {}{facts}  <- {from}, {step}",
            r.ident(),
            sources[r.source].name,
            if r.stage == SHC_STAGE_FRAGMENT { "fs" } else { "vs" },
        )));
    }
    out.extend(
        [
            "",
            "#![allow(clippy::all)]",
            "#![cfg_attr(rustfmt, rustfmt_skip)]",
            "",
            "#[allow(unused_imports)]",
            "use super::{",
            "    ShcAttrib, ShcAttribBinding, ShcAttribMap, ShcBlock, ShcBlockMember, ShcBlocks, ShcPrecompiledLink,",
            "    ShcPrecompiledReflection, ShcPrecompiledRow, ShcSource, ShcUniform, ShcVarying, SHC_STAGE_FRAGMENT,",
            "    SHC_STAGE_VERTEX,",
            "};",
        ]
        .map(String::from),
    );
    #[cfg(feature = "gpu-vivante")]
    out.extend(
        ["#[allow(unused_imports)]", "use super::{pad_io, ShcViv, ShcVivIo, SHC_VIV_MAX_IO};"].map(String::from),
    );
    out.extend(["#[allow(unused_imports)]", "use crate::util::cname;", ""].map(String::from));
    out.push("/// The toolchain mark: which compiler made this table, for which build.".into());
    out.push(format!("pub const SHC_PRECOMPILED_TOOLCHAIN: &str = \"{}\";", rs_str(h.toolchain)));
    out.push("/// The same, NUL-terminated (`shc_precompiled_toolchain()`).".into());
    out.push(format!("pub const SHC_PRECOMPILED_TOOLCHAIN_C: &str = \"{}\\0\";", rs_str(h.toolchain)));
    out.push("/// The GPU and ES level the rows were compiled for (the library refuses a table for another).".into());
    out.push(format!("pub const SHC_PRECOMPILED_GPU: &str = \"{}\";", rs_str(h.gpu)));
    out.push(format!("pub const SHC_PRECOMPILED_ES2: bool = {};", h.es2));
    out.push("/// The source list the table was made from (what a refusal tells the reader to add to).".into());
    out.push(format!("pub const SHC_PRECOMPILED_LIST: &str = \"{}\";", rs_str(h.list)));
    out.push(String::new());

    out.push(format!("pub static SHC_PRECOMPILED_SOURCES: [ShcSource; {}] = [", sources.len()));
    for s in sources {
        out.push("    ShcSource {".into());
        out.push(format!("        name: \"{}\",", rs_str(&s.name)));
        out.push("        text: concat!(".into());
        let lines: Vec<&str> = s.text.split('\n').collect();
        let lines = if s.text.ends_with('\n') { &lines[..lines.len() - 1] } else { &lines[..] };
        for line in lines {
            out.push(format!("            \"{}\\n\",", rs_str(line)));
        }
        out.push("            \"\\0\",".into());
        out.push("        ),".into());
        out.push("    },".into());
    }
    out.push("];".into());
    out.push(String::new());

    let mut entries = Vec::with_capacity(rows.len());
    let mut named: HashSet<String> = HashSet::new();
    /* One reflection per compile: the source, its stage, and whether the
     * compile left its attribute layout to the link. A source's rows are not
     * necessarily neighbours -- a compile, a link and a variant are separate
     * jobs -- so a reflection is remembered by what it belongs to, not by the
     * last group seen.
     *
     * The link is part of the name because a compile that defers is not a
     * variant of the compile the link makes in its place: it has no code, no
     * lists and the sentinel offsets of a record with no layout
     * (deferred_vertex), and the link's record has all of them. Sharing one
     * reflection between the two would give the stub a shape it does not have.
     * Every back end that defers sets the same flag -- the GC7000 and the V3D
     * at more than 16 attribute locations, the PowerVR when its 64
     * vertex-input registers run out -- so this is the one discriminator. */
    let mut refl_of: HashMap<(usize, u32, u8), Vec<ReflField>> = HashMap::new();
    for r in rows {
        let p = print_row_and_reflection(r, sources)?;
        match refl_of.entry((r.source, r.stage, r.shader.layout_at_link)) {
            /* another variant of this compile: every field of its reflection
             * must be the same, or the field belongs in the row, not here */
            Entry::Occupied(had) => {
                let bad = disagreements(had.get(), &p.fields);
                if !bad.is_empty() {
                    return Err(format!(
                        "{}: source {} [{}] has two variants of one compile that disagree on {} of its \
                         reflection, which the table stores once per compile (move the field to the row):\n{}",
                        r.ident(),
                        sources[r.source].name,
                        if r.stage == SHC_STAGE_FRAGMENT { "fs" } else { "vs" },
                        if bad.len() == 1 { "a field" } else { "fields" },
                        bad.join("\n")
                    ));
                }
            }
            Entry::Vacant(v) => {
                if !named.insert(p.rid.clone()) {
                    return Err(format!(
                        "{}: two sources share a reflection name (the first eight bytes of their SHA-256 and \
                         the stage are not enough to name them)",
                        p.rid
                    ));
                }
                out.extend(reflection_static(&p));
                out.push(String::new());
                v.insert(p.fields);
            }
        }
        out.extend(p.row_lists);
        out.extend(row_statics(r)?);
        out.push(String::new());
        entries.push(p.row);
    }
    out.push(format!("pub static SHC_PRECOMPILED_TABLE: [ShcPrecompiledRow; {}] = [", rows.len()));
    for e in entries {
        out.extend(e);
    }
    out.push("];".into());
    let cols: Vec<[String; COLUMNS.len()]> = rows.iter().map(column_entries).collect();
    for (c, (name, ty, doc)) in COLUMNS.iter().enumerate() {
        out.push(String::new());
        out.push(format!("/// {doc}"));
        out.push(format!("pub static {name}: [{ty}; {}] = [", rows.len()));
        for (r, e) in rows.iter().zip(&cols) {
            out.push(format!("    {}, // {}", e[c], r.ident()));
        }
        out.push("];".into());
    }
    out.push(String::new());
    out.push("/// Every link the generator made: the verdict a link takes without the front end.".into());
    out.push(format!("pub static SHC_PRECOMPILED_LINKS: [ShcPrecompiledLink; {}] = [", links.len()));
    let key = |k: &[u8; 32]| format!("[{}]", k.iter().map(|b| format!("0x{b:02x}")).collect::<Vec<_>>().join(", "));
    for l in links {
        let yakogl::shaderc::ShcPrecompiledLink { vs, fs, fs_blend_key, linked_fs } = l;
        out.push(format!(
            "    ShcPrecompiledLink {{ vs: {}, fs: {}, fs_blend_key: 0x{fs_blend_key:x}, linked_fs: {} }},",
            key(vs),
            key(fs),
            key(linked_fs)
        ));
    }
    out.push("];".into());
    out.push(String::new());
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The refusal names every field two rows of one compile disagree on, not
    /// just the first, so one run settles the classification.
    #[test]
    fn disagreements_name_every_field_two_rows_disagree_on() {
        let had = vec![fact("uses_w", "uses_w: 0,".into()), fact("uses_z", "uses_z: 0,".into())];
        let new = vec![fact("uses_w", "uses_w: 1,".into()), fact("uses_z", "uses_z: 1,".into())];
        let bad = disagreements(&had, &new);
        assert_eq!(bad.len(), 2, "{bad:?}");
        assert!(bad[0].starts_with("    uses_w: `uses_w: 0` in one row, `uses_w: 1` in the other"), "{bad:?}");
        assert!(bad.iter().all(|l| !l.contains("stored entries")), "{bad:?}");
    }

    /// A field whose line is the same but whose stored entries are not is a
    /// disagreement too: the table stores the static once, so two rows that
    /// point at one name and fill it differently would write it twice.
    #[test]
    fn disagreements_see_a_static_two_rows_fill_differently() {
        let had = vec![fact("uses_w", "uses_w: 0,".into())];
        let mut new = vec![fact("uses_w", "uses_w: 0,".into())];
        new[0].statics = vec!["static S_VS_VARYINGS: [ShcVarying; 1] = [];".into()];
        let bad = disagreements(&had, &new);
        assert_eq!(bad.len(), 1, "{bad:?}");
        assert!(bad[0].contains("their stored entries differ too"), "{bad:?}");
    }

    /// Rows of the two compiles of one source -- the plain one and the link's
    /// -- are named apart, so a deferred record never borrows the other's
    /// shape ([`emit`]).
    #[test]
    fn a_deferred_row_is_named_apart_from_the_link_s() {
        let row = |layout_at_link: u8| Row {
            key: [0; 32],
            sha: [0; 32],
            source: 0,
            stage: yakogl::shaderc::SHC_STAGE_VERTEX,
            blend_key: 0,
            w_constant: false,
            lod_key: 0,
            if_lowering: 0,
            sensitivity: 0,
            map: None,
            shader: ShcShader { layout_at_link, ..ShcShader::ZERO },
            blocks: Vec::new(),
            members: Vec::new(),
            arrays: Vec::new(),
            origin: (Origin::Alone { src: "a.vert".into() }, Step::Compile),
            code: Vec::new(),
            uniforms: Vec::new(),
            declared: Vec::new(),
            #[cfg(feature = "gpu-vivante")]
            const_contents: Vec::new(),
            #[cfg(feature = "gpu-vivante")]
            const_data: Vec::new(),
        };
        let (plain, deferred) = (reflection_ident(&row(0)), reflection_ident(&row(1)));
        assert_ne!(plain, deferred, "{plain} == {deferred}");
        assert!(deferred.starts_with(&plain), "{deferred}");
    }
}
