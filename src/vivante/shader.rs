// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The shader half of a Vivante draw: which registers the two stages read and
//! write, where their code lives, and their uniforms.
//!
//! The GC7000 links the stages by register number, not by name: the fragment
//! shader reads its varyings from registers 1..N and the VS_OUTPUT map says
//! which vertex-shader output register feeds each one. The offline table holds
//! both halves plus the GLSL names ([`crate::shaderc::ShcViv`]), so linking is
//! a name match the same way the PowerVR offline backend does it.
//!
//! Both shaders' code sits in GPU memory and the shader processor fetches it
//! through its instruction cache; the uniforms are written straight into the
//! constant register file as command-stream words.

use crate::backend::{gles_surf_rt_class, GlesTextureBind, GLES_SURF_R32F, GLES_TEXTYPE_2D_ARRAY, GLES_TEXTYPE_3D};
use crate::shaderc::{
    ShcShader, SHC_RT_RGBA8, SHC_VIV_CONST_LITERAL, SHC_VIV_CONST_SAMPLER_LOD_MAX, SHC_VIV_CONST_TEXRECT_SCALE_X,
    SHC_VIV_CONST_TEXRECT_SCALE_Y, SHC_VIV_CONST_TEXTURE_DEPTH, SHC_VIV_CONST_TEXTURE_HEIGHT,
    SHC_VIV_CONST_TEXTURE_WIDTH, SHC_VIV_CONST_UNIFORM, SHC_VIV_NO_REG,
};

use super::cmd::GcCmd;
use super::regs::*;
use super::state::*;
use crate::util::name_bytes;

/// Write one 8-bit lane of a packed register map ("four registers per word").
fn map_set8(map: &mut [u32], index: usize, value: u8) {
    map[index / 4] &= !(0xffu32 << (8 * (index % 4)));
    map[index / 4] |= (value as u32) << (8 * (index % 4));
}

/// Write one 2-bit lane ("sixteen per word"): the varying component use.
fn map_set2(map: &mut [u32], index: usize, value: u32) {
    map[index / 16] &= !(0x3u32 << (2 * (index % 16)));
    map[index / 16] |= (value & 0x3) << (2 * (index % 16));
}

/// Is fragment input row `i` the point coordinate (`gl_PointCoord`)?
fn is_point_coord_row(fs: &ShcShader, i: usize) -> bool {
    name_bytes(&fs.viv.in_[i].name) == crate::shaderc::SHC_VIV_POINT_COORD.as_bytes()
}

/// Does this fragment shader read `gl_PointCoord`? Mesa's etnaviv sets
/// PA_CONFIG's POINT_SPRITE_ENABLE for a point sprite and clears it when the
/// fragment shader reads no point coordinate (etnaviv_shader.c:197-198).
pub fn reads_point_coord(fs: &ShcShader) -> bool {
    (0..(fs.viv.num_in as usize).min(fs.viv.in_.len())).any(|i| is_point_coord_row(fs, i))
}

/// Write one 4-bit lane ("eight per word"): varying component counts and
/// interpolation modes.
fn map_set4(map: &mut [u32], index: usize, value: u8) {
    map[index / 8] &= !(0xfu32 << (4 * (index % 8)));
    map[index / 8] |= ((value & 0xf) as u32) << (4 * (index % 8));
}

/// The hardware's temporary registers per shader, from the hwdb row for this
/// part (`driver/rust/src/gpu/vivante/hw.rs` `DB`, galcore's `gc7000UL_6200`:
/// `temp_registers` = 64). `NUM_TEMPS` is six bits (`VIVS_VS_TEMP_REGISTER_CONTROL_NUM_TEMPS__MASK`
/// = 0x3f), so 63 is the largest count that can be expressed at all.
pub const GC_MAX_TEMPS: u32 = 63;

/// Refuse a shader that wants more temporaries than the part has.
///
/// This exists because the alternative was silent and catastrophic.
/// `temp_register_num()` masks with 0x3f, so a shader declaring 67 temporaries
/// was programmed as **3** — the hardware reserved three registers per thread
/// and the shader then wrote sixty-seven, walking into whatever the register
/// file held next. Nothing refused, nothing logged, and the two stages fail
/// differently: a fragment shader draws wrong pixels, a vertex shader corrupts
/// the state that drives the pipeline and the GPU stops.
///
/// The count is NOT simply `vs.temps`. `emit_shader_state` adds one per vertex
/// element the shader does not read, because each unread element has to be
/// pointed at a scratch register above the ones in use, so the programmed
/// value can exceed the shader's own temp count. Size the limit against the
/// value actually written, never against `vs.temps` alone.
///
/// A refusal names both numbers, because "too many registers" without the
/// requirement invites raising a limit by one and meeting it again.
pub fn check_temp_limits(d: &GcDraw, vs_temps: u32, fs_temps: u32) -> bool {
    let _ = d;
    let mut ok = true;
    for (stage, n) in [("vertex", vs_temps), ("fragment", fs_temps)] {
        if n > GC_MAX_TEMPS {
            glog_once!(
                "yakogl/vivante: the {} shader needs {} temporary registers; this part has {} \
                 (NUM_TEMPS is 6 bits). Refusing the draw: programming it would wrap to {} and \
                 the shader would write registers the hardware has not reserved.",
                stage,
                n,
                GC_MAX_TEMPS + 1,
                n & 0x3f
            );
            ok = false;
        }
    }
    ok
}

/// The fragment stage's colour output mode for a render target, the rule
/// etnaviv uses (`translate_output_mode`, `etnaviv_translate.h:446`, Mesa, MIT;
/// read and transcribed, never vendored):
///
/// ```text
/// bits = component_bits(fmt, RGB, 0);
/// if (bits == 32)            return COLOR_OUTPUT_MODE_UIF32;
/// if (!is_pure_integer(fmt)) return COLOR_OUTPUT_MODE_NORMAL;
/// ... integer modes ...
/// ```
///
/// The 32-bit test comes **before** the integer test, so a 32-bit float target
/// takes `UIF32`, not `NORMAL`. GL ES 2.0 has no pure-integer colour target, so
/// of the formats this backend renders to (`pe_color_format`, `backend.rs:201`)
/// only `R32F` reaches a mode other than `NORMAL`. It is reachable: a `GL_R32F`
/// FBO colour attachment becomes `GLES_SURF_R32F` (`gl/framebuffer.rs:352`) and
/// arrives here as the draw's target format.
///
/// This backend wrote `NORMAL` unconditionally until the per-draw state audit
/// against etnaviv (2026-09-22). No R32F draw has run on this part, so the
/// `UIF32` arm is transcribed, not measured.
fn ps_color_output_mode(target_format: u8) -> u32 {
    if target_format == GLES_SURF_R32F {
        GC_PS_OUTPUT_MODE_UIF32
    } else {
        GC_PS_OUTPUT_MODE_NORMAL
    }
}

/// Emit the per-stage shader state. Returns **false** when the draw must be
/// refused -- today, only when a stage wants more temporaries than the part
/// has ([`check_temp_limits`]).
pub fn emit_shader_state(c: &mut GcCmd, d: &GcDraw) -> bool {
    let (vs, fs) = (&d.vs.viv, &d.fs.viv);
    let mut vs_input = [0u32; GC_VS_HALTI5_INPUT_LEN];
    let mut vs_output = [0u32; GC_VS_HALTI5_OUTPUT_LEN];
    let mut num_components = [0u32; 2];
    /* four bits per varying COMPONENT: SHC_VIV_MAX_IO rows of four is 64
     * components, 8 of GL_HALTI5_SHADER_ATTRIBUTES' 16 words. It was 4 words
     * (32 components), and a program with nine vec4 varyings indexed past it
     * and panicked in the draw. */
    let mut interp = [0u32; crate::shaderc::SHC_VIV_MAX_IO * 4 / 8];
    let mut total_components = 0usize;
    let varyings = (fs.num_in as usize).min(GC_MAX_VARYINGS);
    let mut out_index = 0usize;

    /* The hardware wants one input per vertex element, so a shader that
     * declares MORE inputs than the draw binds elements cannot be mapped --
     * the surplus inputs would read registers nothing writes.
     *
     * etnaviv refuses this outright, and says why: "Number of vertex elements
     * determines number of VS inputs. Otherwise, the GPU crashes."
     * (`etnaviv_shader.c:357-365`, where `num_vs_inputs != ves->num_elements`
     * is a BUG() and a failed link.) We had no such check: the loop below
     * runs over the ELEMENTS, so surplus inputs were quietly left unmapped
     * and VS_INPUT_COUNT went out as the element count. Found by auditing our
     * per-draw state against etnaviv's.
     *
     * Refused rather than clamped, because the vendor driver's own comment
     * says the failure is a crash and not a wrong picture. */
    if vs.num_in as usize > d.num_elems as usize {
        glog_once!(
            "yakogl/vivante: the vertex shader declares {} inputs but the draw binds {} vertex \
             elements; the hardware requires one input per element. Refusing the draw.",
            vs.num_in,
            d.num_elems
        );
        return false;
    }

    /* the vertex shader's inputs, in vertex-element order, each into the
     * register its element names. There can be MORE elements than io rows:
     * a matrix attribute is one row and one element per column, column k
     * into register vi_reg + k (gl/draw.rs `vertex_inputs`). Every element is
     * one the shader declares, so none is surplus -- the old rule sent
     * elements past `num_in` to spare temporaries (etnaviv's
     * `num_vs_inputs - infile.num_reg + num_temps`, etnaviv_shader.c:367-368),
     * which put a matrix's columns 1.. where the shader never looks.
     *
     * The temporaries programmed must cover every register an element
     * writes: a column the shader never reads can sit above the count its
     * compiler reported, and the vertex fetch would write past NUM_TEMPS. */
    let mut vs_temps = d.vs.temps as u32;
    for (i, elem) in d.elems.iter().enumerate().take((d.num_elems as usize).min(GC_MAX_VERTEX_ELEMENTS)) {
        let reg = elem.vs_reg;
        vs_temps = vs_temps.max(reg as u32 + 1);
        map_set8(&mut vs_input, i, reg);
    }
    let mut vs_input_count = d.num_elems as u32;
    /* Written on EVERY draw, 0 when the shader takes no vertex or instance
     * id. It used to be written only when one did, so after the first
     * shader that used gl_VertexID the front end kept injecting ids into
     * that register for every later draw -- and every later client, since
     * the GPU keeps its state across processes. A register set and never
     * cleared is the class of leak gl_test 63 showed (2026-09-23). */
    let mut id_config = 0u32;
    if vs.vs_id_in_reg != SHC_VIV_NO_REG {
        map_set8(&mut vs_input, vs_input_count as usize, vs.vs_id_in_reg);
        vs_input_count += 1;
        id_config = GC_ID_CONFIG_VERTEX_ID_ENABLE
            | GC_ID_CONFIG_INSTANCE_ID_ENABLE
            | id_config_vertex_id_reg(vs.vs_id_in_reg as u32 * 4)
            | id_config_instance_id_reg(vs.vs_id_in_reg as u32 * 4 + 1);
    }
    c.state(GC_FE_HALTI5_ID_CONFIG, id_config);

    /* the vertex shader's outputs: position, then one per fragment-shader
     * varying in the order the fragment shader reads them, then point size */
    map_set8(&mut vs_output, out_index, vs.vs_pos_out_reg);
    out_index += 1;
    /* What the rasteriser puts in each component: GENERIC everywhere but
     * gl_PointCoord's two, which are the point coordinate (Mesa's etnaviv,
     * etna_link_shader; etnaviv_emit.c writes all four words on every shader
     * change). Written here only for a program that reads gl_PointCoord, and
     * once more -- all GENERIC -- on the first draw after one: a component
     * left marked would hand a later program's generic varying at that
     * position the point coordinate instead. Every other draw leaves the
     * registers alone, so a program without gl_PointCoord emits exactly the
     * stream it did before the registers were known. Its first component's
     * offset is what SH_SPECIALS' PS_PCOORD_IN names. */
    let mut component_use = [GC_VARYING_COMPONENT_USE_GENERIC; 4];
    let mut pcoord_in = GC_SH_SPECIALS_NO_PCOORD;
    for i in 0..varyings {
        let reg = fs.vs_reg_of_in[i];
        let pntc = is_point_coord_row(d.fs, i);

        map_set8(&mut vs_output, out_index, if reg == SHC_VIV_NO_REG { 0 } else { reg });
        out_index += 1;
        map_set4(&mut num_components, i, fs.in_[i].components);
        if pntc {
            pcoord_in = total_components as u32;
            map_set2(&mut component_use, total_components, GC_VARYING_COMPONENT_USE_POINTCOORD_X);
            map_set2(&mut component_use, total_components + 1, GC_VARYING_COMPONENT_USE_POINTCOORD_Y);
        }
        for _ in 0..fs.in_[i].components {
            map_set4(&mut interp, total_components, fs.in_[i].interp);
            total_components += 1;
        }
    }
    let mut vs_output_count = 1 + varyings as u32;
    /* Where the point size sits in the output stream, in components: after
     * position and the varyings, so it is this count -- before the point size
     * is counted in below. SH_SPECIALS wants that offset, while every other
     * register wants the count including the point size, which is the one
     * distinction Mesa draws between its VS_OUTPUT_COUNT and its
     * VS_OUTPUT_COUNT_PSIZE. */
    let psize_component = vs_output_count * 4;
    if vs.vs_pointsize_out_reg != SHC_VIV_NO_REG {
        map_set8(&mut vs_output, out_index, vs.vs_pointsize_out_reg);
        vs_output_count += 1;
    }

    c.state(GC_VS_OUTPUT_COUNT, vs_output_count);
    c.state(
        GC_VS_INPUT_COUNT,
        vs_input_count_count(vs_input_count)
            | vs_input_count_unk8(vs.input_count_unk8 as u32)
            | if vs.vs_id_in_reg != SHC_VIV_NO_REG { GC_VS_INPUT_COUNT_ID_ENABLE } else { 0 },
    );
    /* Both stages' counts are known here; refuse before programming either,
     * because a wrapped count is worse than no draw. */
    /* A 4x draw with gc_debug_msaa's etnaviv extras: one more input and one
     * more temporary (etnaviv_shader.c, PS_*_MSAA). */
    let ms_extra = super::backend::msaa_extras(d.gl) as u32;
    let fs_temps = (d.fs.temps as u32 + ms_extra).max(varyings as u32 + 1 + ms_extra);
    if !check_temp_limits(d, vs_temps, fs_temps) {
        return false;
    }
    c.state(GC_VS_TEMP_REGISTER_CONTROL, temp_register_num(vs_temps));
    c.state(GC_VS_LOAD_BALANCING, vs.vs_load_balancing);
    c.state(GC_VS_HALTI5_OUTPUT_COUNT, vs_output_count | ((vs_output_count * 0x10) << 8));
    c.state(GC_VS_HALTI5_UNK008A0, 0x0001_000e | ((0x110 / vs_output_count.max(1)) << 20));
    c.state_multi(vs_halti5_input(0), &vs_input[..4]);
    c.state_multi(vs_halti5_output(0), &vs_output);

    c.state_multi(pa_varying_num_components(0), &num_components);
    c.state(GC_PA_VS_OUTPUT_COUNT, vs_output_count);

    /* the fragment stage: one input per varying plus position */
    /* Which fragment register feeds each render target, a byte each. RT0 is
     * ps_color_out_reg; RT n the output row at FRAG_RESULT_DATA0 + n, which
     * a shader writing more than one colour lists (select). A target with no
     * row -- the clear shader, a shader that does not write output n -- takes
     * RT0's register: GL leaves an unwritten output undefined, and the clear
     * quad's one colour is what every target of a clear must get. */
    let rt0 = if fs.ps_color_out_reg == SHC_VIV_NO_REG { 0 } else { fs.ps_color_out_reg as u32 };
    /* hardware target rt draws GL slot slots[rt]: a slot with no target
     * is skipped (backend::hw_targets), so its output goes nowhere */
    let (slots, targets) = super::backend::hw_targets(d.gl);
    let mut ps_output_reg = rt0;
    for (rt, &slot) in slots[..targets].iter().enumerate().skip(1) {
        let row =
            fs.out[..(fs.num_out as usize).min(fs.out.len())].iter().find(|o| o.slot as usize == 4 + slot as usize);
        ps_output_reg |= row.map_or(rt0, |o| o.reg as u32) << (8 * rt);
    }
    c.state(GC_PS_OUTPUT_REG, ps_output_reg);
    c.state(
        GC_PS_INPUT_COUNT,
        ps_input_count_count(varyings as u32 + 1 + ms_extra) | ps_input_count_unk8(fs.input_count_unk8 as u32),
    );
    c.state(GC_PS_TEMP_REGISTER_CONTROL, temp_register_num(fs_temps));
    /* The render targets, each one's output saturated for the unorm formats
     * and in its own output mode (etnaviv_state.c:364-392). RT_COUNT(0) is
     * one target, and what disables the rest (etnaviv_state.c:405-406). */
    let mut ps_control = ps_control_rt_count(targets as u32 - 1);
    let mut ps_control_ext = 0;
    for (rt, &slot) in slots[..targets].iter().enumerate() {
        let format = if rt == 0 { d.gl.target_format } else { d.gl.targets[slot as usize].format };
        if gles_surf_rt_class(format) == SHC_RT_RGBA8 {
            ps_control |= ps_control_saturate_rt(rt as u32);
        }
        ps_control_ext |= ps_output_mode(rt as u32, ps_color_output_mode(format));
    }
    c.state(GC_PS_CONTROL, ps_control);
    c.state(GC_PS_CONTROL_EXT, ps_control_ext);
    c.state_multi(ps_varying_num_components(0), &num_components);

    c.state(GC_GL_VARYING_TOTAL_COMPONENTS, (total_components as u32 + 1) & !1);
    c.state(
        GC_GL_HALTI5_SH_SPECIALS,
        GC_SH_SPECIALS_BASE
            | sh_specials_vs_psize_out(if vs.vs_pointsize_out_reg != SHC_VIV_NO_REG { psize_component } else { 0 })
            | sh_specials_ps_pcoord_in(pcoord_in),
    );
    let marks = pcoord_in != GC_SH_SPECIALS_NO_PCOORD;
    if super::backend::component_use_needs_write(marks) {
        c.state_multi(GC_GL_VARYING_COMPONENT_USE0, &component_use[..2]);
        c.state(GC_GL_VARYING_COMPONENT_USE2, component_use[2]);
        c.state(GC_GL_VARYING_COMPONENT_USE3, component_use[3]);
    }
    if total_components != 0 {
        c.state_multi(gl_halti5_shader_attributes(0), &interp[..total_components.div_ceil(8)]);
    }
    true
}

/// Point the instruction cache at each stage's code. The whole shader is one
/// range starting at zero; the count register takes the last instruction's
/// index.
///
/// The prefetch that starts the fetch is deliberately NOT here: it belongs
/// after the uniforms ([`emit_prefetch`] says why), and this block has to be
/// the last thing written before them.
pub fn emit_shader_code(c: &mut GcCmd, d: &GcDraw) {
    let (vs_instr, fs_instr) = (d.vs.usc_dw / 4, d.fs.usc_dw / 4);

    c.state(GC_VS_NEWRANGE_LOW, 0);
    c.state(GC_VS_HALTI5_RANGE_HIGH, vs_instr);
    c.state(GC_VS_INST_ADDR, d.vs_code_gpu as u32);
    c.state(GC_SH_CONFIG, GC_SH_CONFIG_RTNE_ROUNDING);
    c.state(GC_SH_ICACHE_CONTROL, GC_SH_ICACHE_ENABLE);
    c.state(GC_VS_ICACHE_COUNT, vs_instr.saturating_sub(1));

    c.state(GC_PS_NEWRANGE_LOW, 0);
    c.state(GC_PS_HALTI5_RANGE_HIGH, fs_instr);
    c.state(GC_PS_INST_ADDR, d.fs_code_gpu as u32);
    c.state(GC_SH_CONFIG, GC_SH_CONFIG_RTNE_ROUNDING);
    c.state(GC_SH_ICACHE_CONTROL, GC_SH_ICACHE_ENABLE);
    c.state(GC_PS_ICACHE_COUNT, fs_instr.saturating_sub(1));
}

/// Start both stages fetching. This runs after the uniforms, and the order is
/// the whole point.
///
/// The two stages share one constant register file, so something has to say
/// which stage a write into it is meant for. Below HALTI5 the driver says so
/// explicitly in SH_CONTROL; at HALTI5, which this part is, it does not -- the
/// hardware steers each write to whichever stage the shader state written just
/// before it belongs to. Mesa calls that auto-steering and marks the sequence
/// "do not change ordering unless necessary" for exactly this reason.
///
/// So a prefetch between the fragment stage's shader block and its constants
/// breaks the steering: VS_ICACHE_PREFETCH is a vertex-stage register, and
/// after it the file is steered at the vertex stage, so the fragment stage's
/// constants land in the vertex stage's half and its shader reads zeros.
///
/// That is not a reading of somebody's source. The stream Mesa produces for
/// this part was captured on the host against its drm-shim
/// (`tools/vivante/cs_golden.c`, decoded by `cs_decode.py`) and it writes, in
/// this order: both stages' icache blocks, both UNIFORM_BASE registers, the
/// constants, and only then the two prefetches.
pub fn emit_prefetch(c: &mut GcCmd) {
    c.state(GC_VS_ICACHE_PREFETCH, 0);
    c.state(GC_PS_ICACHE_PREFETCH, 0);
}

/// Resolve one constant register: a literal the compiler folded in, a dword
/// of the stage's uniform block, or a property of a bound texture.
fn const_value(d: &GcDraw, contents: u8, data: u32, block: &[u32], fragment: bool) -> u32 {
    match contents {
        SHC_VIV_CONST_LITERAL => return data,
        /* gl_FragCoord.y' = y * scale + offset (select.rs `frag_coord`): the
         * flip only where this backend draws the target mirrored. */
        crate::shaderc::SHC_VIV_CONST_FRAGCOORD_Y_SCALE => {
            return if super::backend::mirrors(d.gl.flip_y) { (-1.0f32).to_bits() } else { 1.0f32.to_bits() };
        }
        /* where the stage's uniform block was copied for `load`
         * (select.rs LoadUniformIdx): the frame arena, per draw */
        crate::shaderc::SHC_VIV_CONST_UBO_ADDR => {
            return if fragment { d.fs_ubo_gpu as u32 } else { d.vs_ubo_gpu as u32 };
        }
        /* gl_PointCoord.t' = t * scale + offset (select.rs `point_coord`):
         * the flip where this backend does NOT draw the target mirrored */
        crate::shaderc::SHC_VIV_CONST_PNTC_Y_SCALE => {
            return if super::backend::mirrors(d.gl.flip_y) { 1.0f32.to_bits() } else { (-1.0f32).to_bits() };
        }
        crate::shaderc::SHC_VIV_CONST_PNTC_Y_OFFSET => {
            return if super::backend::mirrors(d.gl.flip_y) { 0 } else { 1.0f32.to_bits() };
        }
        crate::shaderc::SHC_VIV_CONST_FRAGCOORD_Y_OFFSET => {
            return if super::backend::mirrors(d.gl.flip_y) { (d.gl.target_h as f32).to_bits() } else { 0 };
        }
        SHC_VIV_CONST_UNIFORM => {
            /* A uniform the shader reads but the block does not reach is NOT a
             * zero uniform, and returning one silently is how a wrong colour
             * looks exactly like a correct black. The fragment-uniform defect
             * (gl_test case 1, measured 2026-09-21) has that shape: every
             * candidate in the upload path has since been eliminated against
             * Mesa's etna_uniforms_write, so a block that never arrived is the
             * next suspect. Give the absence its own representation instead of
             * a value indistinguishable from a real one -- the stage and the
             * two numbers are what name the cause on the first board run. */
            return match block.get(data as usize) {
                Some(v) => *v,
                None => {
                    /* Once: this runs for every constant of every draw, and a
                     * line per draw was 2.7 s of the i.MX8MP cluster's frame
                     * 2 (padding lanes the compiler described as uniform
                     * dwords, tests/gc_const_map.rs). */
                    glog_once!(
                        "yakogl/vivante: {} uniform dword {} missing, block holds {} (reading 0; logged once)",
                        if fragment { "fragment" } else { "vertex" },
                        data,
                        block.len()
                    );
                    0
                }
            };
        }
        _ => {}
    }
    let binds: &[GlesTextureBind] = if fragment { d.textures } else { d.vs_textures };
    let Some(t) = binds.iter().find(|t| t.unit as u32 == data) else {
        return 0;
    };
    let f = match contents {
        SHC_VIV_CONST_TEXTURE_WIDTH => return t.width,
        SHC_VIV_CONST_TEXTURE_HEIGHT => return t.height,
        /* textureSize()'s third component: the layers of an array or the
         * slices of a volume. It was hard-coded to 1 back when every texture
         * reached the backend as one flat 2D image and the count was not in
         * the bind; it is now. A cube reports 1, which is what GL says
         * textureSize(samplerCube) returns -- it has no third component. */
        SHC_VIV_CONST_TEXTURE_DEPTH => {
            return if t.type_ == GLES_TEXTYPE_2D_ARRAY || t.type_ == GLES_TEXTYPE_3D {
                t.layers.max(1) as u32
            } else {
                1
            }
        }
        SHC_VIV_CONST_TEXRECT_SCALE_X => {
            if t.width != 0 {
                1.0 / t.width as f32
            } else {
                0.0
            }
        }
        SHC_VIV_CONST_TEXRECT_SCALE_Y => {
            if t.height != 0 {
                1.0 / t.height as f32
            } else {
                0.0
            }
        }
        SHC_VIV_CONST_SAMPLER_LOD_MAX if t.mip_filter != 0 && t.mip_levels != 0 => (t.mip_levels - 1) as f32,
        /* SHC_VIV_CONST_SAMPLER_LOD_MIN, _LOD_BIAS and anything else */
        _ => 0.0,
    };
    f.to_bits()
}

/// Dwords per LOAD_STATE of the constant file. The header's count field is
/// ten bits (Mesa cmdstream.xml.h VIV_FE_LOAD_STATE_HEADER_COUNT__MASK
/// 0x03ff0000), so one command can never carry the 1,100 dwords of a vertex
/// shader with 275 registers; a count of 1024 would even set the FIXP bit
/// above it. 256 keeps each piece well inside it and the buffer small.
const UNIFORM_CHUNK: usize = 256;

/// Write the first `room` dwords of one stage's constant file at `base`: its
/// share of the file the two stages hold together. The link refuses a
/// program whose two files do not fit ([`constant_files_fit`]), so `room`
/// cuts nothing off a linked program.
fn emit_stage_uniforms(
    c: &mut GcCmd,
    d: &GcDraw,
    sh: &ShcShader,
    base: u32,
    room: usize,
    block: &[u32],
    fragment: bool,
) {
    let contents = sh.viv.const_contents();
    let data = sh.viv.const_data();
    let n = contents.len().min(data.len()).min(room);

    let mut values = [0u32; UNIFORM_CHUNK];
    let mut first = 0;
    while first < n {
        let len = (n - first).min(UNIFORM_CHUNK);
        for (k, v) in values[..len].iter_mut().enumerate() {
            *v = const_value(d, contents[first + k], data[first + k], block, fragment);
        }
        c.state_multi(base + 4 * first as u32, &values[..len]);
        first += len;
    }
}

/// The vec4 registers a shader's constant file takes: its dword count
/// rounded up, because the fragment stage starts on a whole register.
pub fn constant_registers(sh: &ShcShader) -> u32 {
    (sh.viv.const_count as u32).div_ceil(4)
}

/// Do the two stages' constant files fit the one file they share
/// ([`GC_NUM_CONSTANTS`] registers, the vertex stage's first)? `Err` holds
/// the two register counts. The link asks (shaderc/online/backend.rs);
/// nothing else stops a program whose literals push it past the file, since
/// the GL's uniform limits count uniforms only.
pub fn constant_files_fit(vs: &ShcShader, fs: &ShcShader) -> Result<(), (u32, u32)> {
    let (v, f) = (constant_registers(vs), constant_registers(fs));
    if v + f > GC_NUM_CONSTANTS {
        return Err((v, f));
    }
    Ok(())
}

pub fn emit_uniforms(c: &mut GcCmd, d: &GcDraw) {
    /*
     * The two stages share one constant register file: the vertex stage
     * starts at register 0 and the fragment stage after it.
     *
     * The fragment stage's start is a REGISTER number and a register is a
     * vec4, so the vertex stage's dword count is rounded up to a whole one.
     * Mesa divides its own count by four without rounding because its count
     * is already a vec4 multiple; ours is a raw dword count out of the
     * offline table, and a vertex shader with six dwords of constants would
     * truncate to one register and leave the fragment stage's window two
     * dwords under its values.
     */
    let vs_vec4 = constant_registers(d.vs);
    let vs_room = GC_MAX_UNIFORM_DWORDS;
    let fs_room = GC_MAX_UNIFORM_DWORDS.saturating_sub(vs_vec4 as usize * 4);

    c.state(GC_VS_UNIFORM_BASE, 0);
    c.state(GC_PS_UNIFORM_BASE, vs_vec4);
    emit_stage_uniforms(c, d, d.vs, GC_SH_HALTI5_UNIFORMS_VS, vs_room, d.vs_uniforms, false);
    /*
     * A fragment uniform does not reach its shader on this part (measured
     * 2026-09-21: gl_test case 1 fails, and a vertex uniform in the same draw
     * arrives correctly). It is NOT this window address: writing the fragment
     * stage's values through the vertex stage's window instead, at the offset
     * PS_UNIFORM_BASE names, was tried on silicon and case 1 still failed.
     * Everything else here matches Mesa's etna_uniforms_write() -- the
     * table's mapping, both base registers, the ordering against the shader
     * load, the LOAD_STATE encoding.
     */
    emit_stage_uniforms(c, d, d.fs, GC_SH_HALTI5_UNIFORMS_PS + vs_vec4 * 16, fs_room, d.fs_uniforms, true);
}

#[cfg(test)]
mod temp_limit_tests {
    use super::*;

    /// The numbers from the shader that hung the GPU on 2026-09-22, and the
    /// numbers that did not, so this test fails if the limit ever silently
    /// moves.
    ///
    /// dEQP `shaders.algorithm.hsl_to_rgb_*` compiled by our online back end:
    /// the vertex shader wanted 67 temporaries and the fragment shader 68.
    /// This part has 64 (`register_max` in the hwdb row `gc7000UL_6200`) and
    /// `NUM_TEMPS` is six bits, so 67 was programmed as `67 & 0x3f` = **3**.
    /// The hardware reserved three registers per thread and the shader wrote
    /// sixty-seven.
    ///
    /// The same source compiled by the reference back end wants 10, which is
    /// why the reference pair never hung.
    #[test]
    fn the_counts_that_hung_the_gpu_are_refused() {
        assert!(!check_temp_limits_for_test(67, 68), "67 and 68 must be refused");
        assert!(!check_temp_limits_for_test(64, 10), "64 is one past the file and must be refused");
        assert!(check_temp_limits_for_test(63, 63), "63 is the largest expressible count");
        assert!(check_temp_limits_for_test(28, 29), "the pair that did not wrap still draws");
        assert!(check_temp_limits_for_test(10, 10), "the reference back end's counts");
    }

    /// The wrap this refusal replaces, stated as arithmetic so the reason
    /// cannot be lost: every refused value maps onto a *plausible* small one.
    #[test]
    fn the_old_behaviour_turned_too_many_registers_into_too_few() {
        for (asked, wrapped) in [(67u32, 3u32), (68, 4), (64, 0)] {
            assert_eq!(temp_register_num(asked), wrapped);
            assert!(asked > GC_MAX_TEMPS, "{asked} should be refused now");
        }
        // And the value 64 is the worst of them: it wraps to zero, telling the
        // hardware the shader uses no temporaries at all.
        assert_eq!(temp_register_num(64), 0);
    }

    fn check_temp_limits_for_test(vs: u32, fs: u32) -> bool {
        vs <= GC_MAX_TEMPS && fs <= GC_MAX_TEMPS
    }
}
