<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# imx95_fw

**Question.** Does the Mali-G310's CSF firmware boot from our side, and what
does it answer: the GLB interface version (1.5.0 expected: the file is the
T6's bytes), the group count and stride, the CS features word, CSG 0's
suspend sizes? The pre-study's B3.

**What it does.** `yd_imx95_fw_up` (`board::imx95_gpu::fw_up`): the GPU's
power-on over SCMI (as `imx95_ident`'s), `GPU_ID`, the soft reset, the L2,
AS0 over the firmware's sections, the load, the MCU's start, the interface,
the global configuration and one `PING`, the NanoPC-T6's Z3 code after the
i.MX 95's power-on; then CSG 0's control block read back. Then it stops: no
group is started, no queue, no job. The firmware is
`driver/src/fw/mali_csffw_blob.h` (`driver/tools/mkmalifw.sh` from the
pinned `MALI_CSFFW`, never committed), each of its seven loaded sections
backed by static storage (the shared one `__nocache`), as `t6_fw`'s.

**Expected lines** (illustrative):

```
imx95_fw: image 282624 bytes, 7 loaded sections, sha256 a27847ea..., timer 24000000 Hz
imx95_fw: rc 0 step 0, 41 ms
imx95_fw: power on: level 2 (800000 kHz), gate 800000000 Hz
imx95_fw: glb version = 1.5.0 (0x01050000)
imx95_fw: groups = 8 stride 0x...
imx95_fw: cs0 features = 0x...
imx95_fw: csg0 suspend size = ... protm ...
imx95_fw: PASS GLB 1.5.0, 8 groups, CS features 0x..., suspend .../...
```

**Reading it.** `FAIL step N (...)` names the step (`YD_T6_FW_STEP_*`):

- 3, the power-on, identity, reset or L2: the `power on stopped` line (an
  SCMI refusal, as `imx95_ident`'s) or the `GPU_ID ... reset ... L2` line;
  run `imx95_ident` first.
- 4, AS0: the MMU's enable or a flush (`stop`, `refusal` as
  `YD_T6_MMU_STOP_*`).
- 5, `csf::boot` refused (`refusal` is the `CsfRefusal`; the record after it
  says how far the MCU came: `MCU_STATUS`, the acknowledges).
- 7, the GLB version is not the image header's: the firmware answered, with
  another interface; the `glb version` line has it.
- `csg0 suspend size = not read`: the boot ran but the interface could not
  be read back (`refusal` is the `CsfRefusal` of the discovery).

The GPU stays powered until the PSCI reset.

**Build**: `west build -p always -b verdin_imx95/mimx9596/a55 -DYAKOGL_WERROR=ON -d build/imx95_fw yakogl/tools/probes/verdin_imx95/imx95_fw`
after the header is made.
