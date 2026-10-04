# What a draw writes on the GC7000, and where YakoGL's state goes

Preparation for M6 of `imx8mp-backend-plan.md`, written from Mesa's etnaviv
driver on this host (`src/gallium/drivers/etnaviv/etnaviv_emit.c`,
`etnaviv_state.c`, `etnaviv_texture_desc.c`) and the register descriptions in
`src/etnaviv/hw/`. It was written before any i.MX8MP was in the lab, and it is
still a map rather than a measurement: what YakoGL resolves per draw (the draw
struct the PowerVR backend consumes) against the registers a Vivante backend has
to write, so that backend could be written from a concrete list rather than
discovered register by register. The board arrived on 2026-09-20 and the backend
built from this map now draws on it, so most of what is described here has been
exercised — but the register meanings below remain Mesa's, not readings taken
from silicon. Where a value has actually been measured, the dated sections of
`imx8mp-backend-plan.md` say so; that document, not this one, is the record of
what the hardware did.

The shape of the work is different from the PowerVR. There is no firmware and no
tiling: the CPU writes a flat register file through `LOAD_STATE` commands in a
command buffer, the front end fetches it, and a frame is one command stream.
Most GL state that the PowerVR lowers into shader variants is real hardware
state here.

## The command stream

`LOAD_STATE` writes N consecutive registers from the stream
(`cmdstream.xml.h`); Mesa's `etna_set_state()` is one register,
`etna_set_state_reloc()` one register holding an address the kernel patches. A
draw is `DRAW_INSTANCED` (or `DRAW_PRIMITIVES` on older cores), and a submit
ends with an `EVENT` command whose id the interrupt reports, which is the fence.

etnaviv tracks dirty groups (`ETNA_DIRTY_*`) and emits only what changed, in a
fixed order, because the hardware wants some registers written together. A first
backend can emit everything on every draw and optimise later; the order matters
more than the minimality.

## State groups

### Render target (`ETNA_DIRTY_FRAMEBUFFER`)

| what | registers |
| --- | --- |
| colour surface, stride, format | `PE_COLOR_ADDR` (reloc), `PE_COLOR_STRIDE`, `PE_COLOR_FORMAT` |
| depth surface, stride, format | `PE_DEPTH_ADDR` (reloc), `PE_DEPTH_STRIDE`, `PE_DEPTH_CONFIG` |
| depth range | `PE_DEPTH_NEAR`, `PE_DEPTH_FAR`, `PE_DEPTH_NORMALIZE` |
| multi-target config | `PE_RT_CONFIG_4`, `PE_RT_CONFIG_8`, `PE_MEM_CONFIG` |
| tile status (fast clear) | `TS_MEM_CONFIG`, `TS_COLOR_SURFACE_BASE`, `TS_COLOR_STATUS_BASE`, `TS_COLOR_CLEAR_VALUE`, and the depth equivalents |

YakoGL's window surface is linear XRGB8888; etnaviv's tiled layouts and the tile
status buffer are an optimisation to leave out of a first backend.

### Blend and colour mask (`ETNA_DIRTY_BLEND`, `BLEND_COLOR`)

`PE_ALPHA_CONFIG`, `PE_ALPHA_OP`, `PE_ALPHA_BLEND_COLOR`, `PE_COLOR_FORMAT`
(the colour mask is its `COMPONENTS` field), `PE_LOGIC_OP`, `PE_DITHER`, and on
HALTI5 the per-target `PE_HALTI5_RT_ALPHA_CONFIG`, `PE_HALTI5_RT_COLORMASK`.

This is the big difference from the PowerVR backend, where blending and colour
masks are compiled into fragment-shader variants: here they are state, so the
draw struct's blend fields map to registers and the shader table loses a
dimension.

### Depth and stencil (`ETNA_DIRTY_ZSA`)

`PE_DEPTH_CONFIG` (test, write, near and far), `PE_STENCIL_CONFIG`,
`PE_STENCIL_OP`, `PE_STENCIL_CONFIG_EXT`, `PE_STENCIL_CONFIG_EXT2`,
`PE_HDEPTH_CONTROL`. Two-sided stencil is the `EXT` registers.

### Rasteriser (`ETNA_DIRTY_RASTERIZER`)

`PA_CONFIG` (cull face, fill mode, front winding), `PA_LINE_WIDTH`,
`PA_WIDE_LINE_WIDTH0/1`, `PA_POINT_SIZE`, `PA_SYSTEM_MODE`, and
`SE_DEPTH_SCALE` / `SE_DEPTH_BIAS` for polygon offset.

### Viewport and scissor (`ETNA_DIRTY_VIEWPORT`, `SCISSOR_CLIP`)

`PA_VIEWPORT_SCALE_X/Y/Z` and `PA_VIEWPORT_OFFSET_X/Y/Z` (X and Y in fixed
point, `EMIT_STATE_FIXP` in Mesa), `SE_SCISSOR_LEFT/TOP/RIGHT/BOTTOM` and
`SE_CLIP_RIGHT/BOTTOM`. Mesa rounds the scissor outwards and intersects it with
the viewport; the scissor is in the same fixed-point format.

### Vertex input (`ETNA_DIRTY_VERTEX_ELEMENTS`, `VERTEX_BUFFERS`)

`FE_VERTEX_ELEMENT_CONFIG(n)` describes each attribute (format, normalised,
size, stream, offset); `NFE_VERTEX_STREAMS_CONTROL` and
`FE_VERTEX_STREAM_CONTROL` the strides; `NFE_VERTEX_STREAMS_BASE_ADDR` or
`FE_VERTEX_STREAM_BASE_ADDR` (relocs) the buffers;
`NFE_GENERIC_ATTRIB_CONFIG0/1` and `NFE_GENERIC_ATTRIB_SCALE` the constant
attributes. Indices are `FE_INDEX_STREAM_BASE_ADDR` (reloc) plus
`FE_INDEX_STREAM_CONTROL` for the type (`UNSIGNED_CHAR`, `UNSIGNED_SHORT` or
`UNSIGNED_INT`, so 32-bit indices are native here, unlike the PowerVR).

Unlike the PowerVR, non-float attributes are a hardware format here, so
YakoGL's per-draw float conversion of byte and short attributes can go.

### Shaders (`ETNA_DIRTY_SHADER`)

The instruction words live in memory and the core fetches them through its
instruction cache: `VS_INST_ADDR` and `PS_INST_ADDR` (relocs),
`VS_NEWRANGE_LOW/HIGH`, `PS_NEWRANGE_LOW/HIGH`, `SH_ICACHE_CONTROL` (enable, and
a flush bit per stage), `VS_ICACHE_COUNT` / `PS_ICACHE_COUNT`, `SH_CONFIG`.
Older cores instead write the words into instruction memory (`VS_INST_MEM`),
which this part does not need.

Around them: `VS_START_PC` and `VS_END_PC`, `PS_END_PC`, `VS_INPUT_COUNT`,
`VS_TEMP_REGISTER_CONTROL`, `VS_OUTPUT` / `VS_HALTI5_OUTPUT` (the varying map),
`PS_INPUT_COUNT`, `PS_CONTROL`, `PS_CONTROL_EXT`, `PA_SHADER_ATTRIBUTES`,
`GL_VARYING_NUM_COMPONENTS*`, `GL_VARYING_COMPONENT_USE*`,
`PA_ATTRIBUTE_ELEMENT_COUNT`, `VS_LOAD_BALANCING`.

`yakogl/tools/vivante/` already produces every number in that list except the
addresses: the JSON it emits carries the code words, the temp count, the input
and output registers with their slots, `vs_pos_out_reg`, `ps_color_out_reg` and
`input_count_unk8`.

### Uniforms (`ETNA_DIRTY_CONSTBUF`)

`VS_UNIFORM_BASE` and `PS_UNIFORM_BASE` name the offsets in the constant
register file, and `SH_CONTROL_PS_UNIFORM` gates it on HALTI5; the values are
written as consecutive registers. The tool's `uniforms` array says which
constant register each GL uniform component lands in, which is what the backend
needs to turn a `glUniform*` call into stream words.

### Textures (`ETNA_DIRTY_SAMPLER_VIEWS`)

HALTI5 uses descriptors rather than a register bank per sampler:
`NTE_DESCRIPTOR_ADDR` points at a descriptor holding the surface address,
format, size and mip chain, and `NTE_DESCRIPTOR_SAMP_CTRL0/1` carry the sampler
state (wrap, min, mag, mip, LOD bias, compare). `etnaviv_texture_desc.c` builds
them, and `NTE_DESCRIPTOR_INVALIDATE` is what keeps the descriptor cache honest.

### Caches

`GL_FLUSH_CACHE`, with `COLOR`, `DEPTH`, `TEXTURE` and shader bits, and
`TS_FLUSH_CACHE`, are emitted before the state changes that need them; etnaviv
computes which from the dirty set at the top of `etna_emit_state`. Getting these
wrong is the likely first class of bug on hardware: the picture is right but
stale.

## What this means for the backend contract

`struct yakogl_backend` needs nothing new. The Vivante implementation is a
translation of the same draw struct into the registers above, and three things
get simpler than on the PowerVR: no blend or colour-mask shader variants, no
float conversion of vertex attributes, and no render-pass bookkeeping, because
the GPU is immediate mode. Two get harder: the texture descriptors, and the
cache flushes that the PowerVR firmware handled.
