// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1.10 (BCM2712 D0) draw-index shader-state record: the field
//! values of one draw's programs and the 32 bytes they pack into (pure, plain
//! integers and bools, no `crate::` imports, so a test can include it with
//! `#[path]`, as `limits71.rs` and `format71.rs`).
//!
//! The layout is `GL Shader State Record Draw Index` of Mesa 26.2.1's
//! `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml:1313-1373`
//! ("exclusive to 2712D0 (V3D 7.1.10)"), [`FIELDS`] being that struct's
//! positions. The rules are `docs/notes/v3d71-record-u11.md` section 4:
//!  1. this record only, never the plain 7.1 one; a part that is not 7.1 with
//!     IPREV >= 10 is a named refusal (Mesa's `v3d_device_has_draw_index`,
//!     `common/v3d_device_info.h:93-97`, also accepts `ver > 71`; this table
//!     serves 7.1.10 and later 7.1 only, `docs/notes/v3d71-gl-table.md` s.4);
//!  2. the three programs fill the three code / uniforms groups;
//!  3. `cs_basevertex`, `cs_drawindex`, `vs_basevertex`, `vs_drawindex` are 0
//!     (Mesa never sets them); a draw that needs them is refused;
//!  4. `writes_z` and `disable_ez` are copied from the fragment program and
//!     `Never defer FEP depth writes` is 0 for every program;
//!  5. no early-z in CFG_BITS and the tile mode keeps `Early-Z disable` set:
//!     neither is the record's, the record only carries the two copied bits.
//!
//! The record has no centroid or flat flags (those are the separate
//! `CENTROID_FLAGS` / `FLAT_SHADE_FLAGS` packets); its one related bit is
//! [`FragmentDesc::uses_center_w`] (real pixel centre W in addition to
//! centroid W2).

#![allow(dead_code)]

/// The record's size in bytes (`G:4411` `_length 32`), 32-byte aligned in the
/// indirect buffer.
pub const RECORD_SIZE: usize = 32;

/// How one field is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Uint,
    /// A `minus_one="true"` uint: the field holds the value minus one.
    UintMinusOne,
    /// A shader address. The 29-bit code addresses start 3 bits above the
    /// three flag bits that share their first word, and hold address bits
    /// 3..31; the uniforms addresses are the whole 32 bits.
    Address,
}

/// One packet-XML field: its name, first bit, width and kind.
#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub name: &'static str,
    pub start: u32,
    pub size: u32,
    pub kind: Kind,
}

const fn f(name: &'static str, start: u32, size: u32, kind: Kind) -> Field {
    Field { name, start, size, kind }
}

/// Indices into [`FIELDS`]; each is a field of [`Record`].
pub mod ix {
    pub const POINT_SIZE: usize = 0;
    pub const ENABLE_CLIPPING: usize = 1;
    pub const CS_VID: usize = 2;
    pub const CS_IID: usize = 3;
    pub const CS_BIID: usize = 4;
    pub const CS_BASEVERTEX: usize = 5;
    pub const CS_DRAWINDEX: usize = 6;
    pub const VS_VID: usize = 7;
    pub const VS_IID: usize = 8;
    pub const VS_BIID: usize = 9;
    pub const VS_BASEVERTEX: usize = 10;
    pub const VS_DRAWINDEX: usize = 11;
    pub const Z_WRITES: usize = 12;
    pub const TURN_OFF_EZ: usize = 13;
    pub const CENTER_W: usize = 14;
    pub const SAMPLE_RATE: usize = 15;
    pub const ANY_PID: usize = 16;
    pub const INSERT_PID: usize = 17;
    pub const TURN_OFF_SCOREBOARD: usize = 18;
    pub const SCOREBOARD_WAIT: usize = 19;
    pub const DISABLE_IMPLICIT: usize = 20;
    pub const NO_PRIM_PACK: usize = 21;
    pub const NEVER_DEFER_FEP: usize = 22;
    pub const VARYINGS: usize = 23;
    pub const CS_OUT_SIZE: usize = 24;
    pub const CS_OUT_MIN: usize = 25;
    pub const CS_IN_SIZE: usize = 26;
    pub const CS_IN_MIN: usize = 27;
    pub const VS_OUT_SIZE: usize = 28;
    pub const VS_OUT_MIN: usize = 29;
    pub const VS_IN_SIZE: usize = 30;
    pub const VS_IN_MIN: usize = 31;
    pub const FS_CODE: usize = 32;
    pub const FS_THREADABLE: usize = 33;
    pub const FS_FINAL: usize = 34;
    pub const FS_NAN: usize = 35;
    pub const FS_UNIFORMS: usize = 36;
    pub const VS_CODE: usize = 37;
    pub const VS_THREADABLE: usize = 38;
    pub const VS_FINAL: usize = 39;
    pub const VS_NAN: usize = 40;
    pub const VS_UNIFORMS: usize = 41;
    pub const CS_CODE: usize = 42;
    pub const CS_THREADABLE: usize = 43;
    pub const CS_FINAL: usize = 44;
    pub const CS_NAN: usize = 45;
    pub const CS_UNIFORMS: usize = 46;
}

use Kind::*;

/// Every field of `GL Shader State Record Draw Index`, in [`ix`] order, with
/// the XML's name, `start` (bits; `Nb` of the XML is N*8) and `size`.
pub const FIELDS: [Field; 47] = [
    f("Point size in shaded vertex data", 0, 1, Bool),
    f("Enable clipping", 1, 1, Bool),
    f("Vertex ID read by coordinate shader", 2, 1, Bool),
    f("Instance ID read by coordinate shader", 3, 1, Bool),
    f("Base Instance ID read by coordinate shader", 4, 1, Bool),
    f("cs_basevertex", 5, 1, Bool),
    f("cs_drawindex", 6, 1, Bool),
    f("Vertex ID read by vertex shader", 7, 1, Bool),
    f("Instance ID read by vertex shader", 8, 1, Bool),
    f("Base Instance ID read by vertex shader", 9, 1, Bool),
    f("vs_basevertex", 10, 1, Bool),
    f("vs_drawindex", 11, 1, Bool),
    f("Fragment shader does Z writes", 12, 1, Bool),
    f("Turn off early-z test", 13, 1, Bool),
    f("Fragment shader uses real pixel centre W in addition to centroid W2", 15, 1, Bool),
    f("Enable Sample Rate Shading", 16, 1, Bool),
    f("Any shader reads hardware-written Primitive ID", 17, 1, Bool),
    f("Insert Primitive ID as first varying to fragment shader", 18, 1, Bool),
    f("Turn off scoreboard", 19, 1, Bool),
    f("Do scoreboard wait on first thread switch", 20, 1, Bool),
    f("Disable implicit point/line varyings", 21, 1, Bool),
    f("No prim pack", 22, 1, Bool),
    f("Never defer FEP depth writes", 23, 1, Bool),
    f("Number of varyings in Fragment Shader", 24, 8, Uint),
    f("Coordinate Shader output VPM segment size", 32, 4, Uint),
    f("Min Coord Shader output segments required in play in addition to VCM cache size", 36, 4, Uint),
    f("Coordinate Shader input VPM segment size", 40, 4, Uint),
    f("Min Coord Shader input segments required in play", 44, 4, UintMinusOne),
    f("Vertex Shader output VPM segment size", 48, 4, Uint),
    f("Min Vertex Shader output segments required in play in addition to VCM cache size", 52, 4, Uint),
    f("Vertex Shader input VPM segment size", 56, 4, Uint),
    f("Min Vertex Shader input segments required in play", 60, 4, UintMinusOne),
    f("Fragment Shader Code Address", 67, 29, Address),
    f("Fragment Shader 4-way threadable", 64, 1, Bool),
    f("Fragment Shader start in final thread section", 65, 1, Bool),
    f("Fragment Shader Propagate NaNs", 66, 1, Bool),
    f("Fragment Shader Uniforms Address", 96, 32, Address),
    f("Vertex Shader Code Address", 131, 29, Address),
    f("Vertex Shader 4-way threadable", 128, 1, Bool),
    f("Vertex Shader start in final thread section", 129, 1, Bool),
    f("Vertex Shader Propagate NaNs", 130, 1, Bool),
    f("Vertex Shader Uniforms Address", 160, 32, Address),
    f("Coordinate Shader Code Address", 195, 29, Address),
    f("Coordinate Shader 4-way threadable", 192, 1, Bool),
    f("Coordinate Shader start in final thread section", 193, 1, Bool),
    f("Coordinate Shader Propagate NaNs", 194, 1, Bool),
    f("Coordinate Shader Uniforms Address", 224, 32, Address),
];

/// The record's field values, one per record field (the unpacked struct of
/// the generated `V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX`). The `min_*_in_play`
/// input fields hold the logical value (packed minus one).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub point_size_in_shaded_vertex_data: bool,
    pub enable_clipping: bool,
    pub vertex_id_read_by_coordinate_shader: bool,
    pub instance_id_read_by_coordinate_shader: bool,
    pub base_instance_id_read_by_coordinate_shader: bool,
    pub cs_basevertex: bool,
    pub cs_drawindex: bool,
    pub vertex_id_read_by_vertex_shader: bool,
    pub instance_id_read_by_vertex_shader: bool,
    pub base_instance_id_read_by_vertex_shader: bool,
    pub vs_basevertex: bool,
    pub vs_drawindex: bool,
    pub fragment_shader_does_z_writes: bool,
    pub turn_off_early_z_test: bool,
    pub fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2: bool,
    pub enable_sample_rate_shading: bool,
    pub any_shader_reads_hardware_written_primitive_id: bool,
    pub insert_primitive_id_as_first_varying_to_fragment_shader: bool,
    pub turn_off_scoreboard: bool,
    pub do_scoreboard_wait_on_first_thread_switch: bool,
    pub disable_implicit_point_line_varyings: bool,
    pub no_prim_pack: bool,
    pub never_defer_fep_depth_writes: bool,
    pub number_of_varyings_in_fragment_shader: u32,
    pub coordinate_shader_output_vpm_segment_size: u32,
    pub min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size: u32,
    pub coordinate_shader_input_vpm_segment_size: u32,
    pub min_coord_shader_input_segments_required_in_play: u32,
    pub vertex_shader_output_vpm_segment_size: u32,
    pub min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size: u32,
    pub vertex_shader_input_vpm_segment_size: u32,
    pub min_vertex_shader_input_segments_required_in_play: u32,
    pub fragment_shader_code_address: u32,
    pub fragment_shader_4_way_threadable: bool,
    pub fragment_shader_start_in_final_thread_section: bool,
    pub fragment_shader_propagate_nans: bool,
    pub fragment_shader_uniforms_address: u32,
    pub vertex_shader_code_address: u32,
    pub vertex_shader_4_way_threadable: bool,
    pub vertex_shader_start_in_final_thread_section: bool,
    pub vertex_shader_propagate_nans: bool,
    pub vertex_shader_uniforms_address: u32,
    pub coordinate_shader_code_address: u32,
    pub coordinate_shader_4_way_threadable: bool,
    pub coordinate_shader_start_in_final_thread_section: bool,
    pub coordinate_shader_propagate_nans: bool,
    pub coordinate_shader_uniforms_address: u32,
}

impl Record {
    /// The value of field `i` of [`FIELDS`] as an integer (bools 0/1).
    pub fn value(&self, i: usize) -> u32 {
        let b = |v: bool| v as u32;
        match i {
            ix::POINT_SIZE => b(self.point_size_in_shaded_vertex_data),
            ix::ENABLE_CLIPPING => b(self.enable_clipping),
            ix::CS_VID => b(self.vertex_id_read_by_coordinate_shader),
            ix::CS_IID => b(self.instance_id_read_by_coordinate_shader),
            ix::CS_BIID => b(self.base_instance_id_read_by_coordinate_shader),
            ix::CS_BASEVERTEX => b(self.cs_basevertex),
            ix::CS_DRAWINDEX => b(self.cs_drawindex),
            ix::VS_VID => b(self.vertex_id_read_by_vertex_shader),
            ix::VS_IID => b(self.instance_id_read_by_vertex_shader),
            ix::VS_BIID => b(self.base_instance_id_read_by_vertex_shader),
            ix::VS_BASEVERTEX => b(self.vs_basevertex),
            ix::VS_DRAWINDEX => b(self.vs_drawindex),
            ix::Z_WRITES => b(self.fragment_shader_does_z_writes),
            ix::TURN_OFF_EZ => b(self.turn_off_early_z_test),
            ix::CENTER_W => b(self.fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2),
            ix::SAMPLE_RATE => b(self.enable_sample_rate_shading),
            ix::ANY_PID => b(self.any_shader_reads_hardware_written_primitive_id),
            ix::INSERT_PID => b(self.insert_primitive_id_as_first_varying_to_fragment_shader),
            ix::TURN_OFF_SCOREBOARD => b(self.turn_off_scoreboard),
            ix::SCOREBOARD_WAIT => b(self.do_scoreboard_wait_on_first_thread_switch),
            ix::DISABLE_IMPLICIT => b(self.disable_implicit_point_line_varyings),
            ix::NO_PRIM_PACK => b(self.no_prim_pack),
            ix::NEVER_DEFER_FEP => b(self.never_defer_fep_depth_writes),
            ix::VARYINGS => self.number_of_varyings_in_fragment_shader,
            ix::CS_OUT_SIZE => self.coordinate_shader_output_vpm_segment_size,
            ix::CS_OUT_MIN => self.min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size,
            ix::CS_IN_SIZE => self.coordinate_shader_input_vpm_segment_size,
            ix::CS_IN_MIN => self.min_coord_shader_input_segments_required_in_play,
            ix::VS_OUT_SIZE => self.vertex_shader_output_vpm_segment_size,
            ix::VS_OUT_MIN => self.min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size,
            ix::VS_IN_SIZE => self.vertex_shader_input_vpm_segment_size,
            ix::VS_IN_MIN => self.min_vertex_shader_input_segments_required_in_play,
            ix::FS_CODE => self.fragment_shader_code_address,
            ix::FS_THREADABLE => b(self.fragment_shader_4_way_threadable),
            ix::FS_FINAL => b(self.fragment_shader_start_in_final_thread_section),
            ix::FS_NAN => b(self.fragment_shader_propagate_nans),
            ix::FS_UNIFORMS => self.fragment_shader_uniforms_address,
            ix::VS_CODE => self.vertex_shader_code_address,
            ix::VS_THREADABLE => b(self.vertex_shader_4_way_threadable),
            ix::VS_FINAL => b(self.vertex_shader_start_in_final_thread_section),
            ix::VS_NAN => b(self.vertex_shader_propagate_nans),
            ix::VS_UNIFORMS => self.vertex_shader_uniforms_address,
            ix::CS_CODE => self.coordinate_shader_code_address,
            ix::CS_THREADABLE => b(self.coordinate_shader_4_way_threadable),
            ix::CS_FINAL => b(self.coordinate_shader_start_in_final_thread_section),
            ix::CS_NAN => b(self.coordinate_shader_propagate_nans),
            ix::CS_UNIFORMS => self.coordinate_shader_uniforms_address,
            _ => panic!("record field index out of range"),
        }
    }

    /// The 32 packed bytes, little endian, by [`FIELDS`]. The values must be
    /// in range (what [`describe`] guarantees).
    pub fn pack(&self) -> [u8; RECORD_SIZE] {
        let mut out = [0u8; RECORD_SIZE];
        for (i, fd) in FIELDS.iter().enumerate() {
            let v = self.value(i);
            match fd.kind {
                Bool | Uint => put(&mut out, fd.start, fd.size, v),
                UintMinusOne => put(&mut out, fd.start, fd.size, v.wrapping_sub(1)),
                // a 29-bit code address field holds address bits 3..31
                Address if fd.size == 29 => put(&mut out, fd.start, fd.size, v >> 3),
                Address => put(&mut out, fd.start, fd.size, v),
            }
        }
        out
    }
}

fn put(out: &mut [u8; RECORD_SIZE], start: u32, size: u32, v: u32) {
    for k in 0..size {
        if v >> k & 1 != 0 {
            let bit = start + k;
            out[(bit / 8) as usize] |= 1 << (bit % 8);
        }
    }
}

/// The device revision: Mesa's `devinfo.ver` (42, 71) and the IPREV
/// (HUB_IDENT3 bits 15:8; 10 on the Pi 5 D0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision {
    pub ver: u32,
    pub rev: u32,
}

/// The Pi 5's revision: V3D 7.1.10.
pub const BCM2712_D0: Revision = Revision { ver: 71, rev: 10 };

/// One of the three programs as the record names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Fragment,
    Vertex,
    Coordinate,
}

impl Stage {
    pub const fn name(self) -> &'static str {
        match self {
            Stage::Fragment => "fragment",
            Stage::Vertex => "vertex",
            Stage::Coordinate => "coordinate",
        }
    }
}

/// What every program contributes: where it is and how it runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProgramDesc {
    /// GPU address of the code (8-byte aligned).
    pub code: u32,
    /// GPU address of its uniform stream.
    pub uniforms: u32,
    /// 1, 2 or 4 threads; 4 sets "4-way threadable".
    pub threads: u32,
    /// "start in final thread section" (the compiler's single-seg flag).
    pub single_seg: bool,
    pub propagate_nans: bool,
}

/// A coordinate or vertex program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VertexDesc {
    pub prog: ProgramDesc,
    /// VPM input / output segment sizes (4-bit fields).
    pub vpm_input_size: u32,
    pub vpm_output_size: u32,
    /// "Min ... input segments required in play" (1..=16; Mesa 1) and "... output
    /// ... in addition to VCM cache size" (0..=15; Mesa 0).
    pub min_input_segments: u32,
    pub min_output_segments: u32,
    pub uses_vid: bool,
    pub uses_iid: bool,
    pub uses_base_instance: bool,
    /// The program reads gl_BaseVertex / gl_DrawID: refused (u11 s.4 item 3).
    pub uses_base_vertex: bool,
    pub uses_draw_index: bool,
}

/// The fragment program and the draw state the record carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FragmentDesc {
    pub prog: ProgramDesc,
    /// The compiler's `writes_z` and `disable_ez`, copied as they are.
    pub writes_z: bool,
    pub disable_ez: bool,
    /// Number of varyings the fragment program reads (0..=255).
    pub num_varyings: u32,
    /// Real pixel centre W in addition to centroid W2.
    pub uses_center_w: bool,
    /// Reads gl_PrimitiveID: sets both primitive-id bits.
    pub uses_pid: bool,
    pub lock_scoreboard_on_first_thrsw: bool,
    pub uses_implicit_point_line_varyings: bool,
    pub sample_rate_shading: bool,
}

/// One draw's programs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawDesc {
    pub fs: FragmentDesc,
    pub vs: VertexDesc,
    pub cs: VertexDesc,
    /// The vertex program writes gl_PointSize ("Point size in shaded vertex data").
    pub point_size: bool,
}

/// Named refusals: what this table does not write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The part has no GL Shader State Record Draw Index (not 7.1, or IPREV < 10).
    NoDrawIndexRecord { ver: u32, rev: u32 },
    /// A code address that is not a multiple of 8 (its low bits are flags).
    CodeAddressUnaligned(Stage),
    /// Threads other than 1, 2 or 4.
    BadThreadCount(Stage, u32),
    /// A VPM size or in-play count outside its 4-bit field (inputs 1..=16).
    VpmFieldRange(Stage, &'static str),
    /// More than 255 varyings.
    TooManyVaryings(u32),
    /// A vertex-side program reads gl_BaseVertex (the record's bits stay 0).
    BaseVertex(Stage),
    /// A vertex-side program reads gl_DrawID (the record's bits stay 0).
    DrawIndex(Stage),
}

impl Refusal {
    pub fn message(&self) -> String {
        match *self {
            Refusal::NoDrawIndexRecord { ver, rev } => format!(
                "V3D {}.{}.{} has no GL Shader State Record Draw Index (7.1.10 or later 7.1 only)",
                ver / 10,
                ver % 10,
                rev
            ),
            Refusal::CodeAddressUnaligned(s) => {
                format!("the {} shader's code address is not 8-byte aligned", s.name())
            }
            Refusal::BadThreadCount(s, n) => {
                format!("the {} shader runs {n} threads (1, 2 or 4 only)", s.name())
            }
            Refusal::VpmFieldRange(s, w) => {
                format!("the {} shader's {w} is outside the record's 4-bit field", s.name())
            }
            Refusal::TooManyVaryings(n) => format!("{n} fragment varyings (the record holds at most 255)"),
            Refusal::BaseVertex(s) => format!("the {} shader reads gl_BaseVertex: refused", s.name()),
            Refusal::DrawIndex(s) => format!("the {} shader reads gl_DrawID: refused", s.name()),
        }
    }
}

/// Mesa's `v3d_device_has_draw_index` restricted to V3D 7.1 (the module note).
pub const fn has_draw_index_record(r: Revision) -> bool {
    r.ver == 71 && r.rev >= 10
}

fn check_prog(s: Stage, p: &ProgramDesc) -> Result<(), Refusal> {
    if p.code & 7 != 0 {
        return Err(Refusal::CodeAddressUnaligned(s));
    }
    if !matches!(p.threads, 1 | 2 | 4) {
        return Err(Refusal::BadThreadCount(s, p.threads));
    }
    Ok(())
}

fn check_vertex(s: Stage, v: &VertexDesc) -> Result<(), Refusal> {
    check_prog(s, &v.prog)?;
    if v.vpm_input_size > 15 {
        return Err(Refusal::VpmFieldRange(s, "VPM input segment size"));
    }
    if v.vpm_output_size > 15 {
        return Err(Refusal::VpmFieldRange(s, "VPM output segment size"));
    }
    if !(1..=16).contains(&v.min_input_segments) {
        return Err(Refusal::VpmFieldRange(s, "min input segments in play"));
    }
    if v.min_output_segments > 15 {
        return Err(Refusal::VpmFieldRange(s, "min output segments in play"));
    }
    if v.uses_base_vertex {
        return Err(Refusal::BaseVertex(s));
    }
    if v.uses_draw_index {
        return Err(Refusal::DrawIndex(s));
    }
    Ok(())
}

/// The record's field values for one draw on `rev`, or the first refusal.
pub fn describe(rev: Revision, d: &DrawDesc) -> Result<Record, Refusal> {
    if !has_draw_index_record(rev) {
        return Err(Refusal::NoDrawIndexRecord { ver: rev.ver, rev: rev.rev });
    }
    check_prog(Stage::Fragment, &d.fs.prog)?;
    check_vertex(Stage::Vertex, &d.vs)?;
    check_vertex(Stage::Coordinate, &d.cs)?;
    if d.fs.num_varyings > 255 {
        return Err(Refusal::TooManyVaryings(d.fs.num_varyings));
    }
    Ok(Record {
        point_size_in_shaded_vertex_data: d.point_size,
        enable_clipping: true,
        vertex_id_read_by_coordinate_shader: d.cs.uses_vid,
        instance_id_read_by_coordinate_shader: d.cs.uses_iid,
        base_instance_id_read_by_coordinate_shader: d.cs.uses_base_instance,
        // rule 3: Mesa never sets the four draw-index bits
        cs_basevertex: false,
        cs_drawindex: false,
        vertex_id_read_by_vertex_shader: d.vs.uses_vid,
        instance_id_read_by_vertex_shader: d.vs.uses_iid,
        base_instance_id_read_by_vertex_shader: d.vs.uses_base_instance,
        vs_basevertex: false,
        vs_drawindex: false,
        // rule 4: copied from the fragment program
        fragment_shader_does_z_writes: d.fs.writes_z,
        turn_off_early_z_test: d.fs.disable_ez,
        fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2: d.fs.uses_center_w,
        enable_sample_rate_shading: d.fs.sample_rate_shading,
        any_shader_reads_hardware_written_primitive_id: d.fs.uses_pid,
        insert_primitive_id_as_first_varying_to_fragment_shader: d.fs.uses_pid,
        turn_off_scoreboard: false,
        do_scoreboard_wait_on_first_thread_switch: d.fs.lock_scoreboard_on_first_thrsw,
        disable_implicit_point_line_varyings: !d.fs.uses_implicit_point_line_varyings,
        no_prim_pack: false,
        // rule 4: Mesa's value for every shader, gl_FragDepth included
        never_defer_fep_depth_writes: false,
        number_of_varyings_in_fragment_shader: d.fs.num_varyings,
        coordinate_shader_output_vpm_segment_size: d.cs.vpm_output_size,
        min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size: d.cs.min_output_segments,
        coordinate_shader_input_vpm_segment_size: d.cs.vpm_input_size,
        min_coord_shader_input_segments_required_in_play: d.cs.min_input_segments,
        vertex_shader_output_vpm_segment_size: d.vs.vpm_output_size,
        min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size: d.vs.min_output_segments,
        vertex_shader_input_vpm_segment_size: d.vs.vpm_input_size,
        min_vertex_shader_input_segments_required_in_play: d.vs.min_input_segments,
        fragment_shader_code_address: d.fs.prog.code,
        fragment_shader_4_way_threadable: d.fs.prog.threads == 4,
        fragment_shader_start_in_final_thread_section: d.fs.prog.single_seg,
        fragment_shader_propagate_nans: d.fs.prog.propagate_nans,
        fragment_shader_uniforms_address: d.fs.prog.uniforms,
        vertex_shader_code_address: d.vs.prog.code,
        vertex_shader_4_way_threadable: d.vs.prog.threads == 4,
        vertex_shader_start_in_final_thread_section: d.vs.prog.single_seg,
        vertex_shader_propagate_nans: d.vs.prog.propagate_nans,
        vertex_shader_uniforms_address: d.vs.prog.uniforms,
        coordinate_shader_code_address: d.cs.prog.code,
        coordinate_shader_4_way_threadable: d.cs.prog.threads == 4,
        coordinate_shader_start_in_final_thread_section: d.cs.prog.single_seg,
        coordinate_shader_propagate_nans: d.cs.prog.propagate_nans,
        coordinate_shader_uniforms_address: d.cs.prog.uniforms,
    })
}

/// Describe and pack in one step.
pub fn pack_draw(rev: Revision, d: &DrawDesc) -> Result<[u8; RECORD_SIZE], Refusal> {
    describe(rev, d).map(|r| r.pack())
}
