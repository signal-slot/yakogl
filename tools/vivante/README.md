# YakoGL offline shaders for the Vivante GC7000 (i.MX8MP)

`etna_shc.c` is a host tool built inside a Mesa tree (26.2.1): it turns an
OpenGL-flavoured SPIR-V module into GC7000 instruction words plus the reflection
YakoGL's shader table needs, using Mesa's own etnaviv compiler and hardware
database (the GPU is described by model/revision/product id; no device is needed).
`glsl2vivante.py` rewrites GLSL ES 1.00 into the GLSL 4.60 the tool expects,
runs `glslangValidator -G` and `etna_shc`, and reports what compiled.

Set up once (the tree is not part of this repository):

```sh
cd <mesa-26.2.1>
cp <this dir>/etna_shc.c src/gallium/drivers/etnaviv/
patch -p1 < <this dir>/mesa-etna_shc.patch   # exports etna_get_specs, adds the meson target
meson setup build-etnaviv -Dgallium-drivers=etnaviv -Dvulkan-drivers= -Dplatforms= \
    -Dglx=disabled -Degl=disabled -Dgbm=disabled -Dllvm=disabled -Dtools=
ninja -C build-etnaviv src/gallium/drivers/etnaviv/etna_shc
```

Then `ETNA_SHC=<mesa>/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc python3 glsl2vivante.py`.
On the lab host the tree is `~/pvr-lab/mesa-build/mesa-26.2.1` and the tool is built.

Status (2026-09-16): all 40 shaders under `yakogl/tools/shaders` compile
(`40/40 compiled`); the matrix vertex shader is 4 instructions, FemtoVG's gradient
fragment shader 59, its image filter 369. The tool runs the GL state tracker's
preparation that the driver compiler expects but does not do itself: system
values to intrinsics, rectangle textures, `flrp` lowering. Missing any of those
shows up as an assertion in the driver's liveness pass or an unhandled ALU op.

## The shader table

`gen_shader_table.py` turns that JSON into the offline table the backend looks
shaders up in, `src/shaderc/vivante_shader_blobs.c`: one entry per shader under
`yakogl/tools/shaders`, found by the SHA-256 of the GLSL text the application
passes to `glShaderSource`, exactly as the PowerVR table works. It also writes
`src/vivante/gc_clear_shader.h`, the two internal shaders the backend draws its
clear quad with (`tools/shaders/internal/clear.vert` and `clear.frag`).

```sh
ETNA_SHC=<mesa>/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc \
    python3 tools/vivante/gen_shader_table.py
```

`--append <path under tools/shaders>...` compiles only the named shaders and
adds them after the existing rows, leaving every committed row, symbol and
source entry byte for byte (the header names the toolchain that compiled the
appended rows). That is how a new shader gets a row on a host whose etna_shc
does not reproduce the committed table: this laptop's (e224bebc...) reproduces
none of the 324 rows, so a regeneration there would be a new reference. The
FemtoVG NO_SCISSOR programs were added this way on 2026-10-02.

`--replace <path under tools/shaders>...` recompiles named rows that are
already in the table, in place: each keeps its position, symbol number and
source entry, every other row stays byte for byte, and the header names the
toolchain that compiled the replaced rows. It refuses the result if any other
row changed. That is how a toolchain fix reaches only the rows it changes:
on 2026-10-04 etna_shc started lowering uniforms to UBO 0 as Mesa's GL state
tracker does (st_glsl_to_nir.cpp st_nir_lower_uniforms), so a run-time
index into a uniform array is a `load` from the stage's uniform block
(constant contents UBO_ADDR) instead of `movar` + a relative constant-file
read, which reads element 0 on the i.MX8MP. Compared on all 327 table
sources, the change moves exactly the four rows that held `movar`; two of
them (`es300/dynidx.frag`, `qt3d/e0880dcd5582d4de.frag`) were replaced, and
the two Qt particle vertex shaders are refused by the generator's layout
check (their `ubuf.sizetable` is packed in the GL core's block where Mesa's
offsets assume a vec4 per element) and keep `movar`, an open item.

Two differences from `tools/glsl2usc.sh`, which builds the PowerVR table:
blending and colour masks are pixel-engine state on this GPU, so there are no
blend variants and every fragment shader appears once; and the input list is
the whole `tools/shaders` tree rather than a hand-kept order, because there is
no variant matrix whose order has to stay stable. A shader the toolchain
cannot build is reported and left out rather than failing the table.

`glsl2vivante.py` translates GLSL ES 1.00 and GLSL ES 3.00; anything else is
taken to be the GLSL 450 the PowerVR tool is fed. The ES 3.00 rewrite is the
small one: those sources are already written the modern way, so it raises the
version to 4.60 and gives a location or a binding to each interface
declaration that has not got one, which is what OpenGL SPIR-V insists on.
Locations need not agree between the stages -- the link matches varyings by
name, which is what GLSL ES means by a varying -- but within a stage they must
be unique, so the ones written by hand are read first and stepped over.

Measured 2026-09-21: 326 of 556 sources compile, against 128 before the ES
3.00 path, and the table holds 304 entries against 138. All twelve of
`gl_test`'s ES 3.00 shaders are in it, matched by SHA-256 against the sources
the suite passes to `glShaderSource`. What is still out is the rest of the Qt
Quick and Qt Quick 3D sweeps. `src/vivante/VIVANTE_NOTES.md` documents the ABI
a table entry holds.

Output per shader (JSON, then the disassembly): `code` (instruction words),
`num_temps`, `inputs`/`outputs` (register, gl slot, components), `uniforms` (the
constant register file: contents 2 = a GL uniform component, `data` = its index),
`uniform_names` (name, GL location, vec4 slot / texture unit), `vs_pos_out_reg`,
`ps_color_out_reg`, `input_count_unk8`, `uses_discard`. What the driver-side
backend does with these is `docs/imx8mp-backend-plan.md`.

## The golden command stream: what a working driver tells this part

`cs_golden.c` + `cs_decode.py` + `mesa-cs-golden.patch` answer the question
that reading somebody else's source cannot: **what does the hardware actually
get told?** Mesa's etnaviv driver runs on the host against its own drm-shim,
which answers as the board's GC7000UL and keeps every submit instead of
executing it. No GPU is involved, so the answer costs a second rather than a
board run, and it is not an opinion.

This is the tool to reach for before guessing at a register. It is how the
fragment-uniform bug was found, after three guesses had been spent on it.

### Building it

```sh
cd ~/pvr-lab/mesa-build/mesa-26.2.1
patch -p1 < tools/vivante/mesa-cs-golden.patch     # see "what the patch does"
meson setup <build dir>/mesa-etnaviv-host \
    -Dgallium-drivers=etnaviv -Dvulkan-drivers= -Dtools=drm-shim \
    -Degl=enabled -Dgles1=disabled -Dgles2=enabled -Dglx=disabled \
    -Dplatforms= -Dgbm=disabled -Dllvm=disabled -Dvideo-codecs= \
    -Dshared-glapi=enabled -Dglvnd=disabled -Dbuildtype=debugoptimized
ninja -C <build dir>/mesa-etnaviv-host
```

`-Dglvnd=disabled` is what gives a standalone `libEGL.so`/`libGLESv2.so` to
link against instead of a GLVND vendor library.

### Running it

```sh
build/run_cs_golden.sh 1                  # gl_test case 1's shape
build/run_cs_golden.sh 1 --uniforms-only  # only the constant-file writes
build/run_cs_golden.sh 2                  # a vertex uniform as well
build/run_cs_golden.sh 4                  # a textured quad (gl_test case 2)

build/run_cs_golden.sh 8                   # a depth-tested draw (gl_test case 3)
build/run_cs_golden.sh 12                  # the same, cleared to depth 0.5 (case 13)
build/run_cs_golden.sh 14                  # case 3's five frames, one submit each
build/run_cs_golden.sh 15                  # a depth clear with nothing drawn behind it

build/run_cs_ours.sh 4 --final-state      # the same draw, this backend
```

`run_cs_ours.sh` builds YakoGL for the Linux OSAL and runs `cs_ours.c`, which
makes the same GL calls with the same shader sources, so the offline table's
SHA-256 lookup finds the shaders the board runs. Neither script needs a GPU.

A good golden run prints `renderer: Vivante GC7000 rev 6204` and `drew; gl
error 0x0` before the decode. `cs_decode.py` names registers out of Mesa's own
`state_3d.xml.h`, so the names in a diff are the ones its source uses; point
`--regs` elsewhere if that tree moves. Every line carries the register's
address as well as its name, because banks overlap and a name alone has been
wrong before.

Names come from **two** of Mesa's headers. `state_3d.xml.h` does not carry the
GL block -- `GL_SEMAPHORE_TOKEN`, `GL_STALL_TOKEN` and `GL_FLUSH_CACHE` are in
`state.xml.h` -- and that is the bank a hang is read in, so it was the worst
one to be printing bare addresses for. Adding it (`--regs2`) named eighteen
more registers and, more to the point, corrected three that were being named
*wrongly* by a neighbouring bank's array:

| address | was printed as | actually |
| --- | --- | --- |
| 0x17800 | `NTE_DESCRIPTOR_SAMP_ANISOTROPY[256]` | `NFE_GENERIC_ATTRIB_CONFIG0[0]` |
| 0x17a00 | `NTE_DESCRIPTOR_SAMP_ANISOTROPY[384]` | `NFE_GENERIC_ATTRIB_SCALE[0]` |
| 0x17a80 | `NTE_DESCRIPTOR_SAMP_ANISOTROPY[416]` | `NFE_GENERIC_ATTRIB_CONFIG1[0]` |

So vertex-attribute state was being read as texture-sampler state. Any diff of
attribute registers taken before 2026-09-22 is worth re-reading. The values
were never wrong -- only the names -- which is exactly what makes it dangerous.

### How the two drivers clear depth, and why the pair 8 / 12 exists

Mode 8 is a depth-tested draw and mode 12 is the same draw with
`glClearDepthf(0.5)` in front of it. Running each against both drivers pins
down the one thing a single mode cannot: whether the clear *value* reaches the
depth buffer, as against the depth state merely looking right.

Mesa does not draw a quad for the depth clear. It clears through the resolve
engine, and the value is a literal in the stream, so diffing its two modes
shows it directly:

```
 mode 8    RS_CLEAR_CONTROL  count=5  0x0001eeee 0xffffff00 ...
 mode 12   RS_CLEAR_CONTROL  count=5  0x0001eeee 0x80000000 ...
```

That is the stored D24S8 word, depth in the top 24 bits and stencil in the
bottom 8: `0xffffff` for 1.0 and `0x800000` for 0.5. It is worth writing down
because it is the ground truth for what the buffer should hold, independent of
any theory about the z transform.

This backend clears with a full-viewport quad instead, at
`z = clear_depth * 2 - 1` in clip space with the depth func forced to ALWAYS.
So its two streams are identical -- the clear value is in a vertex buffer, not
in a register -- and the difference shows up only in the memory dump:

```
 mode 8    -1 -1  1  1   1 -1  1  1   -1  1  1  1   1  1  1  1
 mode 12   -1 -1  0  1   1 -1  0  1   -1  1  0  1   1  1  0  1
```

Measured 2026-09-21: both sides are as intended, and the viewport z state the
two drivers program is identical (`PA_VIEWPORT_SCALE_Z` 1.0,
`PA_VIEWPORT_OFFSET_Z` 0.0). Mesa's own comment in `etnaviv_state.c` explains
that pair -- on this part the hardware's viewport z transform is 0..1 to 0..1,
and Mesa folds the -1..1 conversion into the scale and offset it programs, so
1.0 / 0.0 is what the default GL depth range comes out as. Note also that
`nir_lower_clip_halfz` is *not* applied for this GPU: `vs_need_z_div` is
`model < 0x1000`, and the model here is 0x7000.

**Read that second fact carefully, because I did not.** It says the *shader*
does not do the halving. It does not say nothing does. **The hardware does**,
measured on the board 2026-09-22: a depth clear whose quad carried 0.25
directly stored 0.625, which is `(0.25 + 1) / 2` (gl_test case 53, hold code
39; identity would have given 15). So clip z arriving at this part is in
-1..1 and it converts, and anything drawn here -- including an internal clear
quad with a NOP vertex shader -- must arrive in clip space like geometry.

I built a root cause on the difference between "the shader does not" and
"nothing does", and no amount of reading Mesa settled it: both readings are
consistent with every line of its source. One board reading settled it in
four minutes.

`glDepthRangef` is honoured, and by the same algebra Mesa uses, which is worth
writing down because the two expressions look nothing alike. Mesa programs
`SCALE_Z = scale[2] * 2` and `OFFSET_Z = translate[2] - scale[2]` from a
gallium viewport, where a GL range [n, f] gives `scale[2] = (f - n) / 2` and
`translate[2] = (f + n) / 2`. Substitute and they are `f - n` and `n` -- which
is exactly what this backend writes. For the default [0, 1] both come out as
1.0 and 0.0, the pair in every diff above. Case 13's check 10 uses a
non-default range, so this is the part it exercises.

The upshot is that the host side does not explain gl_test case 13's first
check, which is exactly `glClearDepthf(0.5)` being honoured and which fails on
the board. Whatever is wrong is not in the stream, not in the clear quad, and
not in the viewport state.

### Several frames in one mode, and what that caught

Mode 14 replays gl_test case 3 in full: the depth-tested frame, then the
depth test turned off, then `GL_GREATER` with `glClearDepthf(0)`, then the two
polygon-offset frames. Five frames, five submits, so the decode shows not only
each frame's state but what changes between them -- which is the half a
single-frame mode cannot reach.

It earned its keep immediately. Frame 2 turns the depth test off and leaves
`glDepthMask` alone, and the two drivers disagreed:

```
 Mesa   PE_DEPTH_CONFIG  0x05040711     depth func ALWAYS, write bit CLEAR, DISABLE_ZS
 ours   PE_DEPTH_CONFIG  0x00041711     depth func ALWAYS, write bit SET
```

With `GL_DEPTH_TEST` disabled the depth buffer is not written at all, whatever
the mask says; this backend was writing it. Fixed in `emit_pe_state`, after
which frame 2 reads `0x01040711` and the only bit still differing from Mesa is
`SUPER_TILED`, which describes Mesa's depth surface and not ours.

Two things the mode confirms rather than contradicts, both worth having
pinned: the polygon-offset units come out bit-identical to Mesa's
(`SE_DEPTH_BIAS` 0xb5000001 for -4 and 0x35000001 for +4, i.e. `units * 2 /
(2^24 - 1)`), and Mesa's `glClearDepthf(0)` is an RS clear of a literal
`0x00000000`, the same mechanism as the 0.5 clear in mode 12.

### Adding a mode

Not every mode exists on both sides, and that used to be dangerous rather than
merely inconvenient. `cs_ours` had no modes 2 or 3, and `cs_golden` has no 6,
9, 13 or 16 -- and each harness quietly drew **mode 1's shape** instead. So
`run_cs_ours.sh 2` against `run_cs_golden.sh 2` compared our mode-1 stream
with Mesa's real mode 2, and the diff showed a vertex uniform we were
"missing": a driver bug that does not exist, manufactured entirely by the
tool. Both harnesses now refuse a mode they do not implement and print the
list of the ones they have.

A refusal is not always the right guard, though, and a central list cannot
know that. `cs_golden` *does* have a mode 16 branch -- what it does not have is
a context that will accept ETC2, so ES 2.0 rejects the upload with
`GL_INVALID_ENUM`, the binding keeps its default texture and the harness would
happily describe that instead (`SIZE 0x00010001`, `FORMAT 7`, no compression).
A plausible golden that describes nothing is worse than a refusal, because
nothing stops it being diffed against. So that branch checks `glGetError`
after the upload and stops there, naming the reason. The rule this points at:
**a central list can only say whether a branch exists; only the branch knows
whether it can produce a valid answer**, so the guard belongs where the
knowledge is.

Both harnesses therefore answer in their exit code, which is what a script
should test -- the wording is not a contract:

| code | meaning |
| --- | --- |
| 0 | the mode ran; the stream is worth diffing |
| 2 | no such mode in this harness (the other one may have it) |
| 3 | the branch exists but could not produce a valid stream; the reason is printed |

Grepping stderr for a phrase instead is how a check comes to report success for
every mode including the ones that refuse. That happened here, once.


The mode numbers are defined once, in `cs_ours.c`'s "the mode numbers, in one
place" list. Add the row there before writing the code, and take the lowest
free number: the list is kept in order so that a gap is visible.

This matters more than it sounds. Mode 15 was claimed twice in one evening --
a depth clear and an ETC2 texture -- by two people whose commits touched
different lines of the same two files, so git merged them with no conflict and
mode 15 quietly did both things. Neither reading would have meant anything and
nothing would have said why. A textual conflict would have been the kind
outcome.

### A mode whose answer was that there was no stream

Mode 15 is a depth clear with no draw behind it: `glClearDepthf(0.5)`,
`glClear(GL_DEPTH_BUFFER_BIT)`, `glFinish()`. GL has to carry that out at the
finish. Mesa emits a 44-word submit clearing depth to `0x80000000`. This
backend emitted **no command stream at all** -- the dump file was not even
created, which is how the bug announced itself.

`gl_flush_pending_clear` was gated on the colour clear alone, so a depth- or
stencil-only clear stayed pending and was applied to whatever draw came next,
however much later. Fixed in `src/gl_draw.c`, with the colour mask following
the colour clear so that a depth-only clear leaves the colour buffer alone
(`PE_COLOR_FORMAT` 0x00000006 rather than 0x00010f06: the write-mask field is
zero).

Every other mode's stream is byte-identical across that fix, which is the
check worth running whenever the clear path is touched:

```sh
for m in 1 4 8 12 14; do build/run_cs_ours.sh $m | sed 1,2d > before/o$m.txt; done
# ... change something ...
for m in 1 4 8 12 14; do build/run_cs_ours.sh $m | sed 1,2d | diff before/o$m.txt -; done
```

A missing dump now prints `no command stream was submitted` rather than a
traceback, because it is a result and not a broken tool.

### Which differences in a depth-mode diff are expected

A diff of modes 8 / 12 / 14 still shows two dozen registers apart. Most of
them are not bugs, and knowing which is which is the difference between a
diff that is useful and one nobody trusts. Per submit, the expected ones:

| register | why it differs |
| --- | --- |
| `PE_PIPE_COLOR_ADDR`, `PE_PIPE_DEPTH_ADDR` | the two drivers allocate in different heaps; only the slot and the contents compare |
| `PE_PIPE_*_ADDR[1]` | Mesa writes pipe 0 only; we write both, which is inert on a one-pipe part |
| `PA_VIEWPORT_SCALE_Y` | ours is negative: the window surface is y-flipped, Mesa's FBO is not |
| `PE_COLOR_FORMAT` tiling bits | Mesa's colour surface is tiled, ours is linear |
| `PE_LOGIC_OP` dither field, `PE_STENCIL_CONFIG_EXT+2/+3` | Mesa dithers and carries a dither table; we do not dither |
| `PE_MEM_CONFIG`, `TS_*`, `RS_*` | Mesa's tile status and resolve-engine clears, which this backend does not use |
| `PE_STENCIL_*` | we program stencil on every draw; Mesa only when it is enabled |
| `PE_DEPTH_CONFIG` `SUPER_TILED` | describes Mesa's depth surface, not ours; setting it here was measured inert on the board |
| `NFE_VERTEX_STREAMS_CONTROL` stride | we repack, Mesa does not. `cs_ours`'s vertex struct is 32 bytes but a mode using only position and uv fills 24, and the GL core converts such an array into a tightly packed scratch buffer -- so our 24 describes our buffer and Mesa's 32 describes the application's. What has to agree is the *offset* between streams, and `BASE_ADDR[1] = BASE_ADDR[0] + 16` on both. (vivante-vertex2, mode 10) |
| `FE_INDEX_STREAM_CONTROL`, `FE_PRIMITIVE_RESTART_INDEX`, `FE_HALTI5_ID_CONFIG` | Mesa writes all three as zero, we do not write them at all; zero is the reset value. Newly visible only because the GL block gained names -- worth revisiting if anything ever uses primitive restart |
| the clear quad's z | ours carries the clear depth itself; Mesa has no clear quad at all, clearing through the resolve engine instead. The two are not comparable, and the value is in a buffer rather than the stream, so no diff reaches it -- see the intermediate-clear note above |
| `GL_FLUSH_CACHE` 0x03 (DEPTH \| COLOR) | Mesa emits it only ahead of an `RS_KICKER`, to make the resolve engine see what the pixel engine wrote. We clear with a quad through the same pixel engine, so there is no gap to close and nothing missing |

The colour format of the golden's render target used to be on that list for
the wrong reason. It was `GL_RGBA4`, and Mesa derives both `PE_COLOR_FORMAT`
and `PE_LOGIC_OP`'s `SINGLE_BUFFER` from the colour format, so a 16-bit target
put a difference into those two registers that had nothing to do with depth.
The depth modes now use `GL_RGBA8` so that the comparison is like for like;
every other mode keeps `GL_RGBA4`, so readings already taken against it stand.

What is left after all that, and still open:

- **early Z.** Mesa runs these draws early (`RA_EARLY_DEPTH` 0x05000031,
  `PE_DEPTH_CONFIG` with `EARLY_Z` and `DISABLE_ZS`); we run them late
  (0x15000030, `WRITE_DISABLE` set). Late is internally consistent and Mesa
  emits that same combination itself when early is not allowed, so it is a
  difference rather than a known fault.
- **`PE_LOGIC_OP`'s `SINGLE_BUFFER`**: 2 from Mesa, 1 from us. Mesa picks 1
  only for a linear target and 2 otherwise, and ours *is* linear, so 1 may
  well be right -- but it has not been tested on the board either way.

### The blocks the stream only points at

A command stream is half of what the GPU reads. A texture descriptor, a shader
binary, a vertex buffer: the stream carries an address and the hardware
fetches the block from memory. A stream diff is blind to all of it, which is
how a wrong 256-byte texture descriptor hides behind a set of registers that
match perfectly.

**And a dump you read against your own code is not evidence either.** This is
the sharper half of the lesson, learned the hard way on 2026-09-22. A
`glClearDepthf(0.5)` was landing in the depth buffer as 0.0. The command
stream was *correct* -- every depth register matched Mesa -- because the clear
value never appears in a register at all: this backend clears with a quad, so
the value is four floats in a vertex buffer. The stream diff was structurally
incapable of showing it.

The memory dump did show it, twice, and it was read as correct both times. The
quad carried clip-space `0.0` for a 0.5 clear, which is exactly what
`clear_depth * 2 - 1` computes -- so it matched the code, and the code was
what was wrong. Checking a dump against the source that produced it only
confirms the source ran. What settled it was a board reading of the value the
hardware actually stored (`gl_test` case 53, which clears depth and does
nothing else), and that took one run where four hours of diffing had taken
none.

So: **the stream says what the hardware is told, the dump says what the driver
wrote, and neither says what the hardware did.** When those two agree and the
board still disagrees, stop diffing and go and measure.

So both sides dump their memory beside the stream, in one format: per
allocation a device address, a size, a zero, the bytes, padded to 8. Mesa's
shim writes it to `ETNA_SHIM_MEM_DUMP`, this backend's Linux OSAL to
`YAKOGL_MEM_DUMP`, and both runner scripts set them.

```sh
build/run_cs_golden.sh 4 --tex-desc       # Mesa's texture descriptor
build/run_cs_ours.sh   4 --tex-desc       # ours, field by field
```

`--tex-desc` follows every `NTE_DESCRIPTOR_ADDR` into the dump and prints the
descriptor named from Mesa's `texdesc_3d.xml.h` -- the same header its
`etnaviv_texture_desc.c` fills the block through, so the two sides are named
identically. The addresses themselves differ (the two drivers allocate in
different heaps); the slot number and the contents are what compare.

### What the patch does, and why each piece is needed

Four changes, all to the shim, none to the driver:

1. **A `GC7000UL` entry** with the board's ids (model `0x7000`, revision
   `0x6204`, product `0x70003`) and *no* feature words, plus DRM version 1.4
   for that entry only. With 1.4 the driver asks for the product id and looks
   the core up in NXP's own feature database, which is the authority on what
   this part is -- and says **halti 5**, 320 constants. The stock `GC7000L`
   entry is a different part (revision `0x6214`) with hand-copied feature
   words. Only an entry that carries the ids may claim 1.4: the others' param
   arrays stop at `SOFTPIN_START_ADDR` and reading past it is out of bounds.
2. **`GET_PARAM` fails for any core but 0.** The winsys walks core indices
   looking for both a 3D core and an NPU, stopping when a query fails. A shim
   that answers every index alike is walked for ever.
3. **The perfmon queries fail** (with `errno` set -- libdrm retries `EINTR`
   and `EAGAIN`, so a bare -1 is a spin, not a refusal). They are walked until
   one fails, and the stock no-op succeeds for ever.
4. **`GEM_SUBMIT` writes the stream out** to `$ETNA_SHIM_CS_DUMP` instead of
   discarding it, and every buffer the submit references to
   `$ETNA_SHIM_MEM_DUMP`. Those two files are the golden stream and the memory
   it points at. The bytes come out of the shim's memfd at each buffer's own
   device address rather than out of a mapping, because the driver is free to
   have unmapped it by the time it submits.

There is a fifth change outside the etnaviv shim, in `src/drm-shim/drm_shim.c`:
the shim re-resolves two dozen libc symbols with `dlsym(RTLD_NEXT, ...)` on
*every* intercepted call. That is cheap for shader-db; against a whole
EGL/gallium stack the symbol tables are large enough that it never finishes.
Resolving them once fixes it.
