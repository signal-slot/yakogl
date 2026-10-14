<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gl_InstanceID per draw on the PowerVR and the GC7000UL (design note)

**Status (2026-10-14): investigation only, host-checked at `e0eb883d`. No
source was changed.** Plan #144 item `instanceid-design`, a follow-up to
`es3-instanced` (`6730b25`, `docs/handoff/20261014-es3-instanced.md`,
`tests/es3_instanced.rs`).

## Summary

The six dEQP-GLES3 cases
`functional.instanced.{draw_arrays_instanced,draw_elements_instanced}.mixed.{2,4,20}_instances`
use arrays with divisor 0, 1 and 2, and the vertex shader reads
`gl_InstanceID`. Both back ends set `max_instance_divisor` to 1. So
`draw_instanced` (`src/gl/draw.rs:4565`) draws one instance per draw. It feeds
each divisor array as a constant and sets `GlesDraw::instance_id` to the
instance number (`src/gl/draw.rs:4635-4637`).

- **PowerVR: refused.** The draw entry refuses an instance > 0 when the
  program reads `gl_InstanceID`. The host shows `-EINVAL` for instances 1..3.
- **GC7000UL: not refused, drawn wrong.** This is new: the es3-instanced note
  said the GC7000UL refuses too. The refusal in `src/vivante/backend.rs`
  tests `ShcShader::vs_instance_id_reg1`, and only the PowerVR compiler
  sets that field. The GC7000UL online compiler leaves it at 0. Every
  instance is drawn, and `gl_InstanceID` reads 0 in each one (the FE counts
  from 0 in every DRAW_INSTANCED). The image is wrong, and
  `gc_refused_draws` stays 0.
- **Fix, PowerVR:** first use the PDS's own `instance_id_modifier` (Mesa
  `pvr_pds.c`). It adds a constant to the instance number before the
  `gl_InstanceID` write. That costs one PDS instruction and one constant per
  draw, and it fits the per-instance loop as it is. After that, real divisors
  through the PDS stream `multiplier`/`shift`.
- **Fix, GC7000UL:** use the hardware divisor register that the back end
  already writes (`NFE_VERTEX_STREAMS_VERTEX_DIVISOR`, 0x14680, as Mesa
  etnaviv does on HALTI2+). With divisors up to 0xffff on the GPU, the draw is
  one DRAW_INSTANCED and the FE gives the right `gl_InstanceID`. Also make
  the GC7000UL compiler set `vs_instance_id_reg1`, so the refusal works for
  any draw that is still split into instances.

## 1. Where each back end refuses

### PowerVR (`src/pvr/**`)

`src/pvr/backend.rs:4585-4599`, the draw entry (`pvr_backend_draw`), before
any frame work:

```rust
    /* One instance of an instanced draw the GL core draws instance by
     * instance (a divisor past max_instance_divisor): the PDS starts the
     * instance number at 0 for every draw, so a program that reads
     * gl_InstanceID would read 0 for instance_id. Refused by name, not drawn
     * wrong. */
    if d.instance_id != 0 && !d.vs.is_null() && unsafe { (*d.vs).vs_instance_id_reg1 } != 0 {
        ...
            glog!(
                "yakogl/pvr: draw refused: an instance drawn on its own (a divisor past 1) whose program reads \
                 gl_InstanceID"
            );
        ...
        return -EINVAL;
    }
```

Related sites:

- `src/pvr/backend.rs:7530-7531`: `/* the PDS fetches a per-instance array by
  the instance number: divisor 1 */ max_instance_divisor: 1,`
- `src/pvr/backend.rs:4820-4829`: an instanced draw that is too large for one
  kick is split into chunks of instances. A program that reads
  `gl_InstanceID` is never split, because `gl_InstanceID` would restart at 0
  in each chunk (`reads_instance_id`).
- `src/pvr/backend.rs:5548-5561`: the vertex-fetch PDS program is built by
  `pvr_pds_render_vertex_program_ids(..., id_reg_of(vs.vs_vertex_id_reg1),
  id_reg_of(vs.vs_instance_id_reg1), ...)`. There is no way to pass a
  base instance.
- `driver/src/pvr_pds_render_gen.c:93-104`: the generator sets only
  `iterate_instance_id` and `instance_id_register` of Mesa's
  `pvr_pds_vertex_shader_program`. It never sets `instance_id_modifier`.
- The online compiler's PowerVR back end does not refuse. It sets the
  field that the back end tests: `src/shaderc/online/powervr/chain.rs:211-212`
  (`sh.vs_vertex_id_reg1 = reg1(vid)?; sh.vs_instance_id_reg1 = reg1(iid)?;`),
  with the register order from `src/shaderc/online/powervr/abi.rs:266-283`.

### GC7000UL (`src/vivante/**`, `src/shaderc/online/vivante/**`)

`src/vivante/backend.rs:2350-2355`, `draw_one` (`:2340`, called by `gc_draw`):

```rust
    /* one instance of an instanced draw the GL core draws instance by
     * instance: the FE starts the instance ID at 0 for every draw, so a
     * program that reads gl_InstanceID is refused (counted by gc_draw) */
    if gl.instance_id != 0 && !gl.vs.is_null() && unsafe { (*gl.vs).vs_instance_id_reg1 } != 0 {
        return -crate::util::EINVAL;
    }
```

This check never fires for a GC7000UL program (see section 4):

- The GC7000UL compiler gives the id register in
  `ShcViv::vs_id_in_reg` only: `src/shaderc/online/select.rs:3983-3987`
  (`reads_ids` is true for `VertexId` **or** `InstanceId`) and
  `src/shaderc/online/abi.rs:552`. `gl_VertexID` is `.x` of that register and
  `gl_InstanceID` is `.y` (`src/shaderc/online/select.rs:1562-1566`).
- `vs_instance_id_reg1` is set only in `src/shaderc/online/powervr/chain.rs:212`
  (search for `vs_instance_id_reg1` under `src/`). The unit test
  `an_instance_drawn_on_its_own_that_reads_gl_instance_id_is_refused`
  (`src/vivante/backend_tests.rs`) sets the field by hand, so it passes even
  though real programs never set it.
- `src/shaderc/online/vivante/**` (encoder, allocator, ISA) holds no
  instance-ID code and no refusal.

Related sites:

- `src/vivante/backend.rs:2884-2886`: `/* divisor 1, as before the field
  (GcStream carries a divisor; whether the FE takes more than 1 is
  unmeasured) */ max_instance_divisor: 1,`
- `src/vivante/backend.rs:2261`: `GcStream { ..., divisor: u32::from(a.per_instance != 0) }`.
  It ignores `GlesAttrib::divisor`.
- `src/vivante/backend.rs:1156-1158`: `emit_streams` already writes
  `nfe_vertex_streams_vertex_divisor(i)` (0x14680 + 4i, `src/vivante/regs.rs:158-159`)
  for every stream on every draw.
- `src/vivante/shader.rs:199-206`: `GC_FE_HALTI5_ID_CONFIG` puts the vertex
  ID in `vs_id_in_reg*4` and the instance ID in `vs_id_in_reg*4+1`.

## 2. How the hardware and Mesa supply gl_InstanceID and divisors

### PowerVR (Mesa pvr, vendored `third_party/mesa-26.2.1/src/imagination/`, compiled into our PDS generator through `driver/compat/pvr_mesa_pds.c`)

- The VDM sends one index list with an instance count
  (`pvr_vdm_emit_index_list_instanced`, `INDEX_LIST3 = count - 1`,
  `src/pvr/backend.rs:6629`). The PDS vertex program gets the vertex index in
  IR0 and the instance number in IR1. With `iterate_instance_id` it writes IR1
  into the vtxin register with a DOUTW (`vulkan/pds/pvr_pds.c:1636-1660`,
  `SRC0 = input_register1`).
- **`instance_id_modifier`** (`vulkan/pds/pvr_pds.h:414-445`;
  `vulkan/pds/pvr_pds.c:956-975`). One `ADD32` of a data-segment constant
  into IR1, emitted before the stream DMAs and before the DOUTW. Mesa's
  comment says what it is for: *"This instanceID modifier is used when a draw
  array instanced call sourcing from client data cannot fit into vertex
  buffer and needs to be broken down into several draw calls."* That is our
  per-instance loop, and also our chunking in `draw_sized`. Because IR1 is
  changed before the DMAs, per-instance streams are fetched with the changed
  number as well.
- **Divisors** (`struct pvr_pds_vertex_stream`, `vulkan/pds/pvr_pds.h:384-398`:
  `multiplier`, `shift`). `pvr_pds.c:1085-1155` turns them into
  `Iout = (Iin * multiplier) >> (shift + 31)` (a MAD and one or two
  SFTLP64). It uses only `shift` (one SFTLP32) when the divisor is a power of
  two. The Vulkan driver computes the values in
  `vulkan/pds/pvr_pipeline_pds.c:771-875`:
  `shift = (31 - clz(divisor - 1)) + 1`,
  `multiplier = (2^(shift+31) + divisor - 1) / divisor`.
- `base_instance` (`pvr_pds.c:939-990`) adds the base to the per-instance DMA
  index only, not to `gl_InstanceID`. This is Vulkan's firstInstance. It is
  not what we need.

### GC7000UL (Mesa etnaviv, `$CCFARM_DEPS_DIR/mesa`, Mesa 26.3.0-devel)

- Per-stream divisor in hardware on HALTI2+:
  `src/gallium/drivers/etnaviv/etnaviv_screen.c:310`
  (`caps->vertex_element_instance_divisor = VIV_FEATURE(screen, ETNA_FEATURE_HALTI2)`),
  `etnaviv_state.c:812-816` (`NFE_VERTEX_STREAMS_VERTEX_DIVISOR[buffer_idx] = elements[idx].instance_divisor`,
  the raw GL divisor), `etnaviv_emit.c:398-401` (state 0x14680).
- `gl_InstanceID` comes from the FE through `FE_HALTI5_ID_CONFIG`
  (`etnaviv_shader.c:491-493`, `INSTANCE_ID_REG(vs->vs_id_in_reg * 4 + 1)`),
  with one `DRAW_INSTANCED` per draw (`etnaviv_context.c:633-634`).
- There is no base-instance state. `src/etnaviv/hw/state.xml.h:228-234` has
  only the ID_CONFIG enables and registers, and the `DRAW_INSTANCED` command
  (`src/etnaviv/hw/cmdstream.xml.h:248-273`) has an instance count and a
  start *vertex* index. Etnaviv never splits an instanced draw, so it needs
  none. A per-draw `gl_InstanceID` offset on the GC7000UL can only come from
  the shader (a uniform). It cannot come from a register.
- etnaviv does not feed the instance ID through a uniform. Only its
  transform-feedback lowering reads `load_instance_id` in NIR
  (`etnaviv_nir_lower_xfb.c:13`), and that still comes from the FE.

## 3. Options, cheapest first

The GL core needs no change for any option below. `draw_instanced`
already compares each divisor with `max_instance_divisor`
(`src/gl/draw.rs:4590-4596`). It hands `GlesAttrib::divisor` (u16) to a back
end that takes it (`tests/es3_instanced.rs`
`any_divisor_backend_gets_one_draw_with_the_divisors`). It also sets
`GlesDraw::instance_id` per instance (`divisor_1_backend_draws_each_instance_with_its_id_and_values`).
So plan #19 (`src/gl/draw.rs`) is not needed for any option.

### PowerVR

| Rank | Option | What changes | Size | Files | Owner |
|---|---|---|---|---|---|
| **P1** (do first) | PDS `instance_id_modifier` = `GlesDraw::instance_id` | Add a parameter `instance_id_base` to `pvr_pds_render_vertex_program_ids` and set `program.instance_id_modifier` from it. Pass `d.instance_id` at the call site. Remove the refusal. Make `VDATA_DWORDS`/`VCODE_DWORDS` larger (one more constant, maybe padded to an even pair, and one more instruction; today the slice at `backend.rs:5575` is `min(VDATA_DWORDS)` after the generator has written `data_size_dw`, so the buffer must grow before the program does). Test: the PDS data holds the modifier constant, and instance 2 is drawn, not refused. | ~30 C + ~30 Rust + ~60 test lines | `driver/src/pvr_pds_render_gen.{c,h}`, `src/pvr/ffi.rs` (534-550), `src/pvr/ffi_layout_check.c` (253-257), `src/pvr/backend.rs` (4585-4599, 2079-2081, 5554-5575), `src/pvr/backend_tests.rs` (mock at 333) | plan #18 (PowerVR back end; the `driver/src` generator goes with it) |
| P1b (with P1, optional) | Correct chunking for programs that read `gl_InstanceID` | In `draw_sized`, set the modifier to `first` for each chunk. Drop `attribs_advance_instances` for the streams, because the modifier already moves the per-instance DMA index. Drop the `reads_instance_id` exception. | ~20 lines + test | `src/pvr/backend.rs` (4747-4853) | #18 |
| P2 | Per-draw uniform: the compiler reads `gl_InstanceID` from a shared register that the driver writes, like `fs_front_face_off1` | A new `ShcShader` field (ABI change), PowerVR selection of `IoSlot::InstanceId` as a shared-register load, and a per-draw shared-register write in the back end. | ~200 lines | `src/shaderc/online/powervr/{select,chain,abi}.rs`, `src/shaderc/mod.rs`, `src/abi_layout.rs`, `src/abi_layout_check.c`, `src/pvr/backend.rs` | #19 (compiler) + #18. **Not recommended**: the PDS does the same in one instruction (P1). |
| **P3** (full path) | Hardware divisors: stream `multiplier`/`shift` | Add a `divisor` field to `struct pvr_pds_render_stream` and `PvrPdsVertexStream` (layout asserts at `ffi.rs:700`). Compute multiplier/shift as in `pvr_pipeline_pds.c:846-864`. `add_stream` (`backend.rs:7547-7567`) must also compare the divisor before it coalesces. Chunking must use P1's modifier, not a base move (a base move is wrong unless `first % divisor == 0`). Set `max_instance_divisor: 0xffff` (`backend.rs:7531`). Then the mixed cases are one draw, and the emulation is used only for divisors > 0xffff. Tests: the PDS data words for divisors 2, 3 and 4, and a coalescing test. | ~60 C + ~60 Rust + ~100 test lines | `driver/src/pvr_pds_render_gen.{c,h}`, `src/pvr/ffi.rs`, `src/pvr/ffi_layout_check.c`, `src/pvr/backend.rs`, `src/pvr/backend_tests.rs` | #18 |

P1 alone fixes the six cases: instance i gets `gl_InstanceID = i`, and the
divisor values are already constants. P3 removes the 2/4/20 separate draws
(and the 20 frame objects) for divisor > 1. It also moves the
`attrib_divisor` cases (divisor 3) to the GPU path, so those 39 passing cases
must be re-run on the board.

### GC7000UL

| Rank | Option | What changes | Size | Files | Owner |
|---|---|---|---|---|---|
| **V0** (do with V1) | Make the refusal work: give GC7000UL programs `vs_instance_id_reg1` | In selection, record `InstanceId` reads apart from `VertexId` reads (`select.rs:3983-3987`). Set `ShcShader::vs_instance_id_reg1 = vs_id_in_reg + 1` (or any non-zero value) when `gl_InstanceID` is read. Test with real compiler output in `src/vivante/backend_tests.rs`: instance 2 is refused and counted. Without V0, a draw that is still split into instances (divisor > `max_instance_divisor`) is drawn wrong without any message. | ~20 lines + test | `src/shaderc/online/select.rs`, `src/shaderc/online/abi.rs` (and the `ShcShader` fill for the GC7000UL output), `src/vivante/backend_tests.rs` | compiler: plan #19 (the online compiler; `select.rs`/`abi.rs` are not in the files #19 marks as held, so the owner must confirm) + #18 for the test |
| V0' (if the compiler cannot change) | Back-end-only refusal on `viv.vs_id_in_reg != SHC_VIV_NO_REG` | One line at `backend.rs:2353`. It also refuses programs that read only `gl_VertexID`, which are drawn correctly today. That is a regression, so this is a stop-gap only. | 1 line + test | `src/vivante/backend.rs` | #18 |
| **V1** (cheapest real fix) | Hardware divisor | `backend.rs:2261`: `divisor: if a.per_instance != 0 { u32::from(a.divisor.max(1)) } else { 0 }`. `backend.rs:2886`: `max_instance_divisor: 0xffff`. Test: the 0x14680 state words are 2 and 1 for the mixed draw, with one DRAW_INSTANCED of n instances. Then the FE gives `gl_InstanceID` 0..n-1 in one draw. **Not measured**: the comment at 2884 says no one knows if the GC7000UL FE takes a divisor > 1. etnaviv relies on it on every HALTI2+ part, and this GPU is HALTI5 (it uses `FE_HALTI5_ID_CONFIG`). | ~5 lines + ~40 test lines | `src/vivante/backend.rs`, `src/vivante/backend_tests.rs` | #18 |
| V2 (only for divisor > 0xffff) | Per-draw uniform for split draws | The compiler loads `gl_InstanceID` from a constant register (a new `ShcViv` field, ABI change) instead of the FE's `.y`, and the back end writes `instance_id` there for each draw. There is no FE base-instance state (section 2), so there is no register-only way. | ~150-200 lines | `src/shaderc/online/select.rs`, `src/shaderc/mod.rs`, `src/abi_layout.rs`, `src/abi_layout_check.c`, `src/vivante/{backend,shader}.rs` | #19 + #18. Not recommended now: after V1 it only matters for divisors > 65535. |

## 4. Host check (both refusal sites, with the real compiler)

The real back ends need a GPU, so this check uses each back end's in-crate
mock rig (`Rig` in `src/pvr/backend_tests.rs` and
`src/vivante/backend_tests.rs`). A scratch test is added at the end of
each file and removed afterwards (it is not committed). It compiles the dEQP
mixed vertex shader with the online compiler. The source is from
`es3fInstancedRenderingTests.cpp:205-314`, TYPE_FLOAT, 4 instances. The test
copies the compiler's id fields into the rig's shader and calls the real
draw entry for `instance_id` 0..3.

The PowerVR version (put it at the end of `src/pvr/backend_tests.rs`):

```rust
const SCRATCH_MIXED_VS: &str = "#version 300 es\nin highp vec4 a_position;\nin mediump float a_instanceG;\nin mediump float a_instanceB;\nout mediump vec4 v_color;\n\nvoid main()\n{\n    gl_Position = a_position + vec4(float(gl_InstanceID) * 2.0 / 4.0, 0.0, 0.0, 0.0);\n    v_color.r = float(gl_InstanceID)/4.0;\n    v_color.g = a_instanceG;\n    v_color.b = a_instanceB;\n    v_color.a = 1.0;\n}\n\0";

#[test]
fn scratch_instanceid_mixed() {
    let rig = Rig::new();
    let mut real = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 1024];
    let compile = crate::shaderc::online::SHC_ONLINE.compile.expect("online compiler");
    // SAFETY: NUL-terminated source, real output, log of the size given.
    let rc = unsafe {
        compile(crate::shaderc::SHC_STAGE_VERTEX, SCRATCH_MIXED_VS.as_ptr() as *const core::ffi::c_char, &mut real, log.as_mut_ptr(), log.len())
    };
    assert_eq!(rc, 0, "compile");
    std::println!("SCRATCH pvr: mixed VS vs_vertex_id_reg1={} vs_instance_id_reg1={}", real.vs_vertex_id_reg1, real.vs_instance_id_reg1);
    let mut fvs = *rig.fvs;
    fvs.vs_instance_id_reg1 = real.vs_instance_id_reg1;
    for id in 0..4u32 {
        let mut d = rig.draw(3);
        d.vs = &fvs;
        d.instance_id = id;
        std::println!("SCRATCH pvr: instance_id={id} -> rc={}", draw(&d));
    }
    assert_eq!(pvr_backend_flush(), 0);
    std::println!("SCRATCH pvr: logs {:?}", rig.logs().iter().filter(|l| l.contains("gl_InstanceID")).collect::<Vec<_>>());
}
```

The GC7000UL version (at the end of `src/vivante/backend_tests.rs`) is the
same up to the compile. Then:

```rust
    std::println!("SCRATCH viv: mixed VS viv.vs_id_in_reg={} vs_instance_id_reg1={}", real.viv.vs_id_in_reg, real.vs_instance_id_reg1);
    let mut vs = *rig.vs;
    vs.viv.vs_id_in_reg = real.viv.vs_id_in_reg;
    vs.vs_instance_id_reg1 = real.vs_instance_id_reg1;
    let base = gc_refused_draws();
    for id in 0..4u32 {
        let mut d = rig.draw(3);
        d.vs = &vs;
        d.instance_id = id;
        // SAFETY: a live draw.
        std::println!("SCRATCH viv: instance_id={id} -> rc={}", unsafe { gc_draw(&d) });
    }
    std::println!("SCRATCH viv: refused draws {}", gc_refused_draws().wrapping_sub(base));
```

Commands and output at `e0eb883d` (x86_64 host, nightly toolchain):

```text
$ cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --lib scratch_instanceid_mixed -- --nocapture
running 1 test
SCRATCH pvr: mixed VS vs_vertex_id_reg1=0 vs_instance_id_reg1=13
SCRATCH pvr: instance_id=0 -> rc=0
SCRATCH pvr: instance_id=1 -> rc=-22
SCRATCH pvr: instance_id=2 -> rc=-22
SCRATCH pvr: instance_id=3 -> rc=-22
SCRATCH pvr: logs ["yakogl/pvr: draw refused: an instance drawn on its own (a divisor past 1) whose program reads gl_InstanceID"]
test pvr::backend::rig_tests::scratch_instanceid_mixed ... ok

$ cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --lib scratch_instanceid_mixed -- --nocapture
running 1 test
SCRATCH viv: mixed VS viv.vs_id_in_reg=3 vs_instance_id_reg1=0
SCRATCH viv: instance_id=0 -> rc=0
SCRATCH viv: instance_id=1 -> rc=0
SCRATCH viv: instance_id=2 -> rc=0
SCRATCH viv: instance_id=3 -> rc=0
SCRATCH viv: refused draws 0
test vivante::backend::backend_tests::scratch_instanceid_mixed ... ok
```

What this shows:

- PowerVR: the compiler gives `vs_instance_id_reg1` 13. Instance 0 is drawn,
  instances 1..3 return `-EINVAL` (-22), and the log line is printed once. So
  on the AM62P only instance 0 is in the image.
- GC7000UL: the compiler gives `vs_id_in_reg` 3 (`a_position`,
  `a_instanceG` and `a_instanceB` in t0..t2, ids in t3) and
  `vs_instance_id_reg1` 0. No draw is refused. Every instance is drawn with
  the FE's `gl_InstanceID` = 0, so all instances land on top of each
  other at offset 0 with red 0.

The GL-core side is unchanged and green on both builds:

```text
$ cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_instanced
test any_divisor_backend_gets_one_draw_with_the_divisors ... ok
test instanced_draw_leaves_the_arrays_and_the_next_draw_alone ... ok
test divisor_1_backend_draws_each_instance_with_its_id_and_values ... ok
test result: ok. 3 passed; 0 failed
$ cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_instanced
(the same three lines) test result: ok. 3 passed; 0 failed
```

After a fix, the scratch test is a good permanent test. With P1 on the
PowerVR, all four draws return 0 and the PDS data holds the modifier. With V0
on the GC7000UL, instances 1..3 return non-zero and `refused draws` is 3. With
V1 the GL core no longer splits the draw (`max_instance_divisor` 0xffff), so
put the test on `GlesAttrib::divisor` and the 0x14680 state words instead.

## 5. Board re-check (after a fix lands)

The six cases, and the 39 other `instanced` cases, which must stay passing.
Use one image with the filter `dEQP-GLES3.functional.instanced.*` (45 cases)
on each board. Build it as each board's recipe says (ES 3.0 context: both
`CONFIG_DEQP_REPORT_ES2=n` and `CONFIG_YAKOGL_REPORT_ES2=n`).

AM62P (`docs/HANDOFF.md`, the sRGB re-run recipe, has the same shape):

```sh
HEAD8=$(git rev-parse --short=8 HEAD)
printf 'dEQP-GLES3.functional.instanced.*\n' > <img>/instanced.txt
DEQP_MODULE=gles3 DEQP_WORK=<gles3 stage 1> \
  tools/cts/am62p-deqp-images.sh <img>/instanced.txt <img> $HEAD8 > <img>/build.out
TREE_IMAGE_DIR=<img> TREE_TAG=$HEAD8 \
  tools/board.sh am62p tree <img>/instanced.txt records/am62p/instanced-$HEAD8
tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt \
  --statuses records/am62p/instanced-$HEAD8/*/console.txt | grep instanced
```

i.MX8MP (`docs/es3-board-recipe-imx8mp.md` sections 2-3; the table has one
row for this group, `dEQP-GLES3.functional.instanced.*` at line 27 of
`tools/cts/imx8mp-gles3-tree.txt`):

```sh
printf 'dEQP-GLES3.functional.instanced.*\n' > <img>/instanced.txt
tools/cts/imx8mp-deqp-build.sh --gles3 inst-$HEAD8 'dEQP-GLES3.info.*' --table <img>/instanced.txt
export BOARD_HOST=<user@host> BOARD_BOOT=uboot-mmc
TREE_IMAGE=$IMX8MP_OUT/inst-$HEAD8/zephyr/zephyr.bin \
  tools/board.sh imx8mp tree <img>/instanced.txt records/imx8mp/instanced-$HEAD8
tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt \
  --statuses records/imx8mp/instanced-$HEAD8/*/console.txt | grep instanced
```

(Failing GLES3 names on the i.MX8MP may print as `crc:<hex>`; map them with
the table, as `docs/handoff/20261014-imx8mp-gles3-stage1.md` says.)

Expected verdicts:

| Build | AM62P, 6 mixed.{2,4,20} | i.MX8MP, 6 mixed.{2,4,20} | the 39 others (both) |
|---|---|---|---|
| today (`e0eb883d`) | Fail. Each case logs `yakogl/pvr: draw refused: an instance drawn on its own ...` once per boot. | Fail (predicted; no GLES3 board run of this group yet). The image is wrong, with no refusal and `gc_refused_draws` unchanged. | Pass (AM62P measured, `docs/es3-cts-failures.md` Appendix A). i.MX8MP not measured. |
| PowerVR P1 | **Pass** | (unchanged) | Pass |
| PowerVR P1 + P3 | **Pass**, one draw per case | (unchanged) | Pass. The `attrib_divisor` cases (divisor 3/2/1) now use the PDS divide: a failure there points at P3's multiplier/shift. |
| GC7000UL V0 only | (unchanged) | Fail, now refused and counted (`gc_refused_draws` > 0) | Pass |
| GC7000UL V0 + V1 | (unchanged) | **Pass** if the FE takes divisors > 1. If the six still fail **and** some `attrib_divisor` cases also fail, the FE ignores divisor > 1: go back to `max_instance_divisor: 1` and use V2. | Pass |

The `mixed.1_instances` cases pass on every build: one instance takes the
GPU path (`draw_instanced`, `instancecount == 1`). The Pi 4 (V3D,
`max_instance_divisor` 0xffff) is predicted to pass already.
