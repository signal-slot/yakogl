<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1.10: the draw-index shader record and FEP depth writes (U11, host part)

Answers the host-readable part of `docs/notes/pre-study-rpi5.md` U11 (row at
line 580): (1) how the 7.1.10 record `GL Shader State Record Draw Index` fits
our fixed three programs (coordinate, vertex, fragment) per GL program;
(2) whether `Never defer FEP depth writes` changes early-z behaviour for a
`gl_FragDepth` shader. Reading only; nothing here is a board measurement.

Both answers are yes/no negative and they come from files, not inference:

1. **It fits unchanged.** Mesa fills the draw-index record from exactly one
   `cs`/`vs`/`fs` triple per draw, picks the packet by the **device revision**
   at run time (not per shader), and sets none of the four new ID bits
   itself (`V:v3dx_draw.c:477-547`).
2. **The flag changes nothing for `gl_FragDepth` early-z, as Mesa uses it.**
   No Mesa 26.2.1 source ever references the packet field (a grep of the tree
   finds it nowhere but the XML), so Mesa emits it 0 for every shader, its
   `gl_FragDepth` shaders included; what makes those shaders late-tested is
   `turn_off_early_z_test` plus the tile-mode `Early-Z disable` driven by
   `v3d_update_job_ez` (`V:v3dx_draw.c:1007-1010`, `V:v3dx_rcl.c:729-748`).

Prefixes. `X:` is `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml`.
`G:` is `driver/src/v3d/generated/v3d_packet_v71_pack.h`. `M:` is
`third_party/mesa-26.2.1/src/broadcom/common/`. `V:` is Mesa 26.2.1's
`src/gallium/drivers/v3d/`, `src/broadcom/compiler/` and
`src/broadcom/vulkan/` — **not vendored** (`third_party/mesa-26.2.1` has
`src/broadcom/{cle,clif,common,qpu}` only). The tree read is Mesa 26.2.1,
commit `da14d65e4499`, every file compared byte-identical to the tarball
`third_party/mesa-26.2.1/UPSTREAM` names (sha256 `c47e81bd…`), so each `V:`
line is reproducible from that tarball; it is the same source the vendored
copies come from (`M:` and `X:` are from the vendored tree).

## 1. The record: what the packet and Mesa say

| Claim | Citation |
|---|---|
| The draw-index record is a second struct next to the plain 7.1 one, "exclusive to 2712D0 (V3D 7.1.10)", `min_ver="71"` | `X:1313-1314` |
| The plain 7.1 record, for comparison, is `X:1256-1311`; the 4.2 record `X:1197-1254` | `X:1256`, `X:1197` |
| Both 7.1 records are 32 bytes; the generated packer agrees | `G:4194`, `G:4411` (`_length 32`) |
| It adds four bools: `cs_basevertex` bit 5, `cs_drawindex` 6, `vs_basevertex` 10, `vs_drawindex` 11 | `X:1321-1328` |
| Vertex-shader ID reads move to 7, 8, 9 (plain 7.1: 5, 6, 7) | `X:1324-1326`, against `X:1263-1265` |
| Z writes is bit 12, turn off early-z 13, real pixel centre W 15 … no prim pack 22, **Never defer FEP depth writes 23** (plain 7.1: 8, 9, 12 … 19, 20) | `X:1330-1341`, against `X:1267-1278` |
| Everything from byte 3 on (varyings count, the four VPM segment fields, the three code/threadable/NaN/uniform-address groups) has the same offsets in both 7.1 records | `X:1343-1373`, `X:1280-1310` |
| Both records are 7.1 only: the 4.2 record has no `Never defer FEP depth writes` and no draw-index bits; a grep of `v3d_packet.xml` finds the field nowhere else | `X:1278`, `X:1341` |
| Mesa emits the draw-index packet **iff** `v3d_device_has_draw_index`: `ver > 71 \|\| (ver == 71 && rev >= 10)`, and asserts the opposite before the plain packet; the XML cannot branch on revisions, so the choice is at run time, per device, not per shader | `M:v3d_device_info.h:93-97`; `V:v3dx_draw.c:469-477` (comment 470-475), `:552-553`; Vulkan the same way: `V:v3dvx_pipeline.c:392-400`, `V:v3dvx_cmd_buffer.c:2476-2478` |
| Mesa fills **all three stages'** code addresses, VPM sizes and uniforms addresses in the one record from `v3d->prog.cs`, `v3d->prog.vs`, `v3d->prog.fs` — one record per draw, one triple of programs; there is no per-stage record and no fourth stage | `V:v3dx_draw.c:499-518` (also `X:1343-1373`) |
| Mesa **never assigns** the four new bits and never assigns `never_defer_fep_depth_writes` anywhere: a grep of Mesa 26.2.1's whole `src/` finds the generated field names only in the XML field names, and the two record blocks set every other field explicitly. The `cl_emit`/`v3dvx_pack` brace-init zeroes the struct, so all five bits are **0** on every Mesa draw | `V:v3dx_draw.c:477-547` (the D0 block: bits at 478-498, ID reads 539-546, no basevertex/drawindex/never_defer); `V:v3d_cl.h:217-219` (brace-init); `V:v3dvx_pipeline.c:400-440` with `V:v3dv_cl.h:237-243` (same: `base_instance_id_read_by_*` from `uses_biid` at 435/438, but no `cs_basevertex`/`cs_drawindex`/`vs_basevertex`/`vs_drawindex`, no never-defer) |

## 2. Question 1: does it fit "3 programs per GL program"?

**Yes, with no change to the model.** The reasons, each from a file:

1. The record's shape is the same as the 4.2 one in everything the model
   touches: one record per draw holding three code addresses and three
   uniform-stream addresses (`X:1343-1373`; 4.2 `X:1197-1254`). Mesa fills
   those three groups from the three programs of one context (`V:v3dx_draw.c
   :499-518`); our 4.2 builder fills them from the three programs `fs`, `vs`,
   `cs` of one draw (`driver/src/v3d/v3d_job.c:913-946`; the draw struct
   holds exactly `fs`, `vs`, `cs`: `src/v3d/ffi.rs:415-417`). The 7.1 triangle
   frame already does this with the draw-index record and the three programs:
   `driver/src/v3d/v3d_frame71_tri.c:296-330`, with the same three
   `V3dFsProg`/`V3dVsProg` inputs and the compiler's three-program result
   `src/shaderc/online/v3d/chain71.rs:144-145,474-475`, which is how the
   board's `v3d_triangle` passed with ours (`tools/cts/RESULTS.md`, "V3D 7.1:
   Raspberry Pi 5 on Zephyr", `:710`).
2. What the variant adds is four flags that say a **vertex-side** stage may
   read `gl_BaseVertex`/`gl_DrawID` (`X:1321-1328`). They concern the
   coordinate and vertex programs, which our compiler already emits per GL
   program; they do not add a fourth program or a per-draw program choice.
   The model is "a GL program links to a coordinate, vertex and fragment
   program"; the flags are properties of those programs, set from what the
   vertex shader reads — exactly what Mesa does with `uses_vid`/`uses_iid`
   (`V:v3dx_draw.c:539-546`).
3. Our programs read neither (the selector side is not grepped beyond the
   files cited; the builder refuses it): the 4.2 job builder refuses a base
   vertex (`driver/src/v3d/v3d_job.h:28-29`: "refused rather than drawn
   wrong: … a base vertex"). `chain71.rs` has no such field (as stated in
   `docs/notes/v3d71-tri-ours-record.md:70`). So the four bits are 0, which
   is what the 7.1 frame code writes (`v3d_frame71_tri.c:296-312`: the
   comment above the emit says "the D0's draw-index and base-vertex bits
   clear"; `docs/notes/v3d71-triangle-frame.md:120,123`) — and, per section
   1, what Mesa writes too: Mesa 26.2.1 itself never sets the four bits,
   revision 10 hardware or not.
4. Because the bits are all 0 and every other field keeps its meaning, a
   draw-index record is the plain 7.1 record with 4 bits inserted. The
   generated packer does the shift itself: the bit placement is
   `G:4318-4330` (`vs_drawindex` at word 1 bit 3, `fragment_shader_does_z_writes`
   bit 4, `turn_off_early_z_test` bit 5), so the builder sets named fields and
   never numbers a bit (`v3d_frame71_tri.c:300-312`).

The one real consequence for the 7.1 table is a **revision rule, not a
model change**: the draw-index record must be used on 7.1.10 and the plain
one is wrong there. Mesa switches on `v3d_device_has_draw_index`
(`M:v3d_device_info.h:93-97`), a part below 7.1.10 reads the plain layout, and
`docs/notes/v3d71-gl-table.md` section 2 item 2 and section 4 already decide
that the table serves 7.1.10 only and refuses the others by name at init.
`tools/v3d/pi5/README.md` "Differences" item 4 records why a plain-layout CLIF
dump of the board's record shows three flags that look changed (real pixel
centre W, Insert Primitive ID, Disable implicit point/line varyings): the dump
decodes the draw-index record with the plain layout; the values read at the
D0 layout are the shim's. A host check that unpacks a 7.1 record must use
`V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX_unpack` (`G:4414`), as
`docs/notes/v3d71-triangle-frame.md:153` already says.

What the model still cannot do is the draw-index **value**: a program that
reads `gl_DrawID` (a multi-draw) or `gl_BaseVertex` (`glDrawElementsBaseVertex`,
ES 3.2) needs the two bit pairs set and the compiler to read them. Mesa
26.2.1 cannot do that either — it has no path that sets the bits (section 1).
None of the six gl_test cases needs it (`docs/notes/v3d71-triangle-frame.md
:188` leaves it "a case of its own"). The table refuses a base-vertex draw by
the existing 4.2 reason string until then.

## 3. Question 2: `Never defer FEP depth writes` and `gl_FragDepth`

The answer is a fact, not a reading of the flag's name: **in Mesa 26.2.1 the
flag does not participate in any behaviour, because no Mesa source sets it.**
A grep of the whole tree for the generated field name finds nothing (the
string exists only as the XML field name, `X:1278`, `X:1341`); both 7.1 record
blocks leave it at 0 (`V:v3dx_draw.c:477-547`, `V:v3dvx_pipeline.c:400-440`,
brace-init zeroing `V:v3d_cl.h:217-219`, `V:v3dv_cl.h:237-243`) — including
for a fragment shader that writes `gl_FragDepth`. Mesa is correct on the
board with the flag 0 (`tools/cts/RESULTS.md` `:710`), so the flag cannot be
where a `gl_FragDepth` shader's correctness lives. What Mesa uses instead:

| Step of Mesa's `gl_FragDepth` path (Mesa 26.2.1) | Citation |
|---|---|
| A shader that is not early-tested, or reads Z, writes Z from the QPU itself: `c->writes_z = true` | `V:nir_to_vir.c:2081-2098` (the comment names the "passthrough Z value produced from FEP") |
| Writing `gl_FragDepth`: that value goes to the TLB per pixel (`TLB_V42_DEPTH_TYPE_PER_PIXEL`); `writes_z_from_fep` stays **false** | `V:nir_to_vir.c:2102-2108` |
| Not writing it: a passthrough Z (invariant) is written and `writes_z_from_fep = true` | `V:nir_to_vir.c:2109-2113` |
| A shader with no colour write: Mesa flags it as possibly discarding, late path | `V:nir_to_vir.c:2122-2128` |
| Early-test promotion: side-effect-free + colour write ⇒ `early_fragment_tests = !uses_discard \|\| can_earlyz_with_discard` | `V:nir_to_vir.c:2071-2079`; `can_earlyz_with_discard` at `V:v3d_program.c:627-631` |
| `disable_ez = !early_fragment_tests` and `writes_z_from_fep` travel in prog_data | `V:vir.c:1011-1013`; fields `V:v3d_compiler.h:1098-1100` |
| Both reach the record, both 7.1 layouts alike: `fragment_shader_does_z_writes = writes_z`, `turn_off_early_z_test = disable_ez` | `V:v3dx_draw.c:482-485` (D0 block), `:564-575` (plain block) |
| `v3d_update_job_ez`: a shader that **writes its own Z** (`writes_z && !writes_z_from_fep` — the `gl_FragDepth` case) disables early-Z for the whole job | `V:v3dx_draw.c:1007-1010` (function 917-918) |
| The job's EZ state becomes the tile-mode `Early-Z disable`/direction bits in the 7.1 render list | `V:v3dx_rcl.c:729-748`; packet fields `X:1044`, `X:1046` |
| Early-z is **not** in 7.1's `Cfg Bits`: 4.2 has `Early Z enable` (16) and `Early Z updates enable` (17); 7.1 has neither (Z updates enable 15 and Depth-Test Function 12 stay); Mesa's own CFG_BITS emission keeps its two early-z assignments under `#if V3D_VERSION == 42` | `X:836-837` vs `X:849-856`; `V:v3dx_emit.c:346-356` |
| The packet XML gives the flag no description, only the name | `X:1278`, `X:1341` |

Our side, unchanged by all of this and consistent with it:

| Claim | Citation |
|---|---|
| The flag exists only in the two 7.1 records: bit 20 plain, bit 23 draw-index; the generated structs have it as `never_defer_fep_depth_writes` | `X:1278`, `X:1341`; `G:4060`, `G:4273` |
| The record's shader-side early-z pair is `Turn off early-z test` (bit 9 plain, 13 draw-index) and `Fragment shader does Z writes` (8 / 12); our builders copy the compiler's `writes_z` and `disable_ez` into them in both generations | `X:1208-1209`, `X:1267-1268`, `X:1330-1331`; `driver/src/v3d/v3d_job.c:913-915`; `v3d_frame71_tri.c:311-312` |
| Our compiler's rule: a shader that discards, writes no colour or writes `gl_FragDepth` has `disable_ez`; `writes_z = disable_ez \|\| reads_z`; `gl_FragDepth` is the shader's own TLB depth write (`TLB_Z_PER_PIXEL`) in place of the pass-through (`TLB_Z_FROM_FEP`) | `src/shaderc/online/v3d/select.rs:2385-2386`, `:473-480`, comment `:2357-2384` — the same chain the `V:` rows above show in Mesa |
| Our 4.2 builder never enables early-z: "early Z (off: legal, only slower)"; CFG_BITS carries `z_updates_enable` only; the tile mode keeps `Early-Z disable` set unconditionally | `v3d_job.h:28-29`; `v3d_job.c:617-637`; `v3d_job.c:1341-1343` |
| The Mesa triangle references decode `Never defer FEP depth writes` as 0 (and Mesa's own value is 0 for every shader, above); our 7.1 frame writes 0 for it and the draw-index bits | `tools/v3d/pi5/triangle.clif:126`, `tools/v3d/shim71/triangle.clif:126`; `docs/notes/v3d71-triangle-frame.md:132`; `docs/notes/v3d71-tri-ours-record.md:81` |

So: early-z for a `gl_FragDepth` shader does **not** depend on the new flag.
`Turn off early-z test` (set, since our `disable_ez` rule covers a depth
write) keeps the depth test after the shader, and our tile mode keeps
early-Z disabled wholesale, as on 4.2 — the two mechanisms the `V:` table
above shows Mesa relying on. What a 1 on the flag would make the hardware do
differently ("deferred FEP depth writes" is described nowhere in the files
read: not in `X:1278`/`X:1341`, not in any Mesa source line) this note does
not claim, and the table does not need it: Mesa runs correct 7.1 hardware
with 0. The six gl_test cases (5, 58, 59, 66, 67, 102) draw no shader depth
at all (59 is refused by name, `docs/notes/v3d71-gl-table.md` section 5; a
grep of `driver/tools/pvrlab/gl_test.c` finds no `gl_FragDepth`), so no case
can exercise the flag and 0 is right for all six.

## 4. What the 7.1 table must do (for cases71 and backend71)

1. Emit `GL_SHADER_STATE_RECORD_DRAW_INDEX` only, 32 bytes, 32-byte aligned,
   with named fields (the generated packer places the bits; `G:4301-4330`;
   the frame's reserve `v3d_frame71_tri.c:290-293`); never pack the plain 7.1
   record on this table. A part with `ver != 71 || IPREV < 10` is a named
   refusal at init (`docs/notes/v3d71-gl-table.md` section 4;
   `M:v3d_device_info.h:93-97`).
2. Keep the three-program model unchanged: fill the three code, threadable,
   final-section, NaN and uniforms-address groups from `cs`, `vs`, `fs` as
   `v3d_frame71_tri.c:296-330` does, as Mesa does at `V:v3dx_draw.c:499-518`.
   No new program, no new key.
3. Set `cs_basevertex`, `cs_drawindex`, `vs_basevertex`, `vs_drawindex` to 0
   — exactly Mesa's value (section 1) — and refuse a draw that needs them
   (a base vertex, `gl_DrawID`) by the existing 4.2 refusal text
   (`v3d_job.h:28-29`). The hook for later is the vertex program's own report
   of what it reads, next to `uses_vid`/`uses_iid` (`V3dVsProg`,
   `src/v3d/ffi.rs:104`, fields `:112-113`).
4. Copy `writes_z` and `disable_ez` from the compiled fragment program into
   the record (as both builders do), and set **`Never defer FEP depth
   writes` to 0** — Mesa's value for every shader, `gl_FragDepth` included.
   Do not derive it from `writes_z` unless a source or the board shows Mesa's
   zero being wrong on some case.
5. Do not emit early-z CFG_BITS bits at 7.1: the fields do not exist
   (`X:849-856`; Mesa's own emission guards them `#if V3D_VERSION == 42`,
   `V:v3dx_emit.c:346-356`). Keep `Early-Z disable` set in the tile mode as
   the 4.2 builder does (`X:1044`; `v3d_job.c:1341-1343`): early-z stays off
   at 7.1, legal and only slower, so a `gl_FragDepth` shader behaves as on
   4.2. (A later table that *enables* early-z must carry the
   `v3d_update_job_ez` rule — a shader writing its own Z disables the job —
   `V:v3dx_draw.c:917-1014`, `V:v3dx_rcl.c:729-748`; that is not P9's.)
6. Host tests: unpack the emitted record with
   `V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX_unpack` (`G:4414`) and compare it
   to the CLIF of the same shape read at the D0 layout (`tools/v3d/pi5/*.clif`,
   `tools/v3d/pi5/README.md` "Differences" item 4); assert the four
   draw-index bits and `never_defer_fep_depth_writes` are 0 for the six
   cases, and, for a discard or `gl_FragDepth` program, that `writes_z` and
   `disable_ez` are 1 (the rule at `select.rs:2385-2386`, host-pinned by
   `tests/es3_fragdepth_v3d.rs` and `tests/es3_fragdepth_v3d_sim.rs`).

## 5. What would confirm it on the board (for board71)

- **The record rule (items 1 to 3).** Any gl_test case that draws: case 5 is
  the first (`driver/tools/pvrlab/gl_test.c:2338`). The job completing and
  the readback matching shows the D0 layout is read as written (the three
  code addresses sit at the same offsets in both 7.1 layouts,
  `X:1294-1310`/`X:1357-1373`, so a wrong layout moves the flags, not the
  addresses: it shows as wrong decoded flags, the trio
  `tools/v3d/pi5/README.md` "Differences" item 4 lists, not a hang; the
  `v3d_triangle` pass already shows the D0 layout with these values,
  `tools/cts/RESULTS.md:710`).
- **The FEP flag (item 4).** None of the six cases decides it, and Mesa's
  evidence says it stays decided at 0. The cases that *would* decide it, if
  the flag is ever to be revisited: `dEQP-GLES3.functional.shaders.fragdepth.*`
  and `dEQP-GLES3.functional.fragment_ops.depth_stencil.*`, both in the Pi 4
  tree (`tools/cts/pi4-gles3-tree.txt:89`, `:55`), plus the clamp note
  (`select.rs:2383-2384`, "the dEQP fbo.depth write_clamp cases are the
  board's to measure"). On the Pi 5 these belong to P13 (dEQP on the Pi 5),
  not to P9: run a `gl_FragDepth` draw with the flag 0 and 1 and compare the
  readback — a difference with `disable_ez` set would be the flag's whole
  effect. Until then the table keeps 0, which is Mesa's value.

## 6. Left open, for the planner

- What the hardware does differently with the flag at 1 is documented nowhere
  in what was read (the XML line is a bare field name; Mesa never sets it).
  Only the dEQP A/B above or a vendor note can say; the table does not need
  it.
- Our own comments quote a few Mesa line numbers that drift by ~2 lines from
  the real 26.2.1 tree (`select.rs:2374` says `nir_to_vir.c:2120-2126`, the
  block is `2122-2128`; `:472` says `2110-2129`, it is `2109-2128`). Harmless
  to this note (it cites the tree directly), a one-line fix for a future
  docs pass.
