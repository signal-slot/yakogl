<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# imx95_ident

**Question.** What is the Verdin iMX95's GPU: its identity and configuration
registers as our code reads them, after the port's own power-on over SCMI
(the pre-study's B2 GPU half; U5, who releases the GPU's reset; U6, the
G310's configuration)? And do the words `driver/rust/src/gpu/mali/mock.rs`
assumes (`G310_ASSUMED`) hold?

**What it does.**
1. The SM's view of the GPU before, read only: PD 14's state, perf 9's
   level, clock 174's configuration and rate.
2. `yd_imx95_gpu_power_on` (`soc::imx95::gpu_power::power_on`, the port's
   sequence: versions, levels, PD 14 on, the 800 MHz level, gate 174 on, its
   rate). A refusal ends the run before any GPU register is read.
3. One 32-bit read of each identity register: those `gpu::mali::ident`
   reads (GPU_ID, L2/TILER/MEM/MMU_FEATURES, AS_PRESENT, CSF_ID,
   GPU_STATUS, SHADER/TILER/L2_PRESENT, COHERENCY_FEATURES, MCU_STATUS) and
   those panthor reads besides (CORE_FEATURES, THREAD_*, TEXTURE_FEATURES_0
   to 3). Nothing is written to the GPU: no reset, no L2.
4. `yd_imx95_gpu_power_off` (gate 174 off, PD 14 off and read back) and the
   SM's view again.

**Expected lines** (the values to learn; illustrative):

```
imx95_ident: before pd 14 state = 0x40000000 (off)
imx95_ident: power on rc 0: level 2 (800000 kHz), gate 800000000 Hz, asked 800000000 Hz
imx95_ident: reg 0x000 GPU_ID = 0xac740001
imx95_ident: reg 0x100 SHADER_PRESENT_LO = 0x00000003
...
imx95_ident: mock pub const G310: [(usize, u32); 9] = [
imx95_ident: mock     (GPU_ID, 0xac74_0001),
...
imx95_ident: mock ];
imx95_ident: mock pub const G310_ASSUMED: Assumed = Assumed { mmu_features: 0x2830, ... };
imx95_ident: against mock.rs MMU_FEATURES = read 0x00002830, mock 0x00002830 (assumed, same)
imx95_ident: power off rc 0
imx95_ident: after pd 14 state = 0x40000000 (off)
imx95_ident: PASS GPU_ID 0xac740001, SHADER_PRESENT 0x00000003, 0 of 9 mock.rs words differ
```

**Reading it.**
- The `mock` lines, with the `imx95_ident: mock ` prefix cut, are
  `mock.rs`'s tables as read: paste them over `G310` and `G310_ASSUMED`
  (and update `docs/notes/mali-g310-vs-g610.md` for every `DIFFERS`).
- `PASS ... N of 9 mock.rs words differ` with N above 0 is still a pass: the
  probe's purpose is the read. Each `DIFFERS` line names the word.
- `FAIL the GPU's power-on over SCMI stopped (refusal R, status S)`: the
  `power on` line says which message (`message` is protocol << 8 | id,
  0x1104 POWER_STATE_SET, 0x1307 PERFORMANCE_LEVEL_SET, 0x1407
  CLOCK_CONFIG_SET) and the status (DENIED: AP-NS lacks the right, compare
  `imx95_scmi`'s permissions). No GPU register was read.
- `FAIL GPU_ID 0x... is not a Mali-G310`: 0 or 0xffffffff means the block
  did not answer (its reset is not released, U5, or the mix's access
  control); another id is another GPU.
- No line after `power on rc 0` and a WDOG3 reset 60 s later: a GPU read
  hung (the domain reads on but the GPU mix is not out of reset): U5.

**Build**: `west build -p always -b verdin_imx95/mimx9596/a55 -DYAKOGL_WERROR=ON -d build/imx95_ident yakogl/tools/probes/verdin_imx95/imx95_ident`.
