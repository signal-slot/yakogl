<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_tfu

Does the Raspberry Pi 5's V3D 7.1.10 run a TFU (texture formatting unit)
job? Its `HUB_IDENT1` is `0x00081117`, with bit 17 clear, which Linux's
debugfs reads as `TFU: no` (`tools/v3d/pi5/v3d_ident.txt`). The Pi 4's word,
`0x000e1124`, has the bit set, and its TFU works. Nothing in the tree says
whether the bit means "TFU present" on 7.x. `driver/src/v3d/v3d_hw.c`
(`v3d_tfu_run`) and `src/osal/zephyr_v3d.c` refuse a TFU job on it. This
probe runs one 7.1 TFU mip-chain job and reads the result back. Roadmap P10,
ROADMAP stage 5 (issue #7).

```sh
west build -p always -b rpi_5 -DYAKOGL_WERROR=ON -d build/rpi5_tfu yakogl/tools/probes/rpi_5/rpi5_tfu
```

The image has no sources of its own besides `src/main.c`. Its
configuration and board overlay are `rpi5_clear`'s: `CMakeLists.txt` merges
`../rpi5_clear/prj.conf` before this directory's empty `prj.conf`, and uses
`../rpi5_clear/boards/rpi_5.overlay`. The driver files it compiles are
`v3d_hw.c` (power-on, identity, MMU), `v3d_tex.c` with `v3d_tiling_mesa.c`
(Mesa's level layout and texel tiling) and `v3d_tfu71.c` (the 7.1 TFU fill).
It uses them without changing them.

## What it does

- **M1** (as `rpi5_clear`): the mailbox switches the V3D clock, then
  `v3d_power_on` and `v3d_identify` run. The probe prints `HUB_IDENT0..3`,
  `CTL_IDENT0..2` and `MMU_DEBUG_INFO` raw, then `HUB_IDENT1` with its bit 17
  and `HUB_IDENT3`. Before anything is written there, it reads the 7.1 TFU
  block's `CS` (0x700) and `SU` (0x704).
- **M2** (as `rpi5_clear`): one page table over the probe's statics, then
  `v3d_mmu_init`.
- **T1**: a 64x64 RGBA8 raster source holds a pattern in which every channel
  moves with x or y: R = 4x+2, G = 4y+1, B = 4(x^y), A = 255-2(x+y). The
  texture is laid out with `v3d_tex_layout(64, 64, 7, 4)`, which is Mesa's
  `v3d_setup_slices` for a sampled 64x64 RGBA8 texture with 7 levels:

  | Level | Offset | Tiling |
  |---|---|---|
  | 0 | 0x2000 | UIF_NO_XOR, padded height 64 |
  | 1 | 0x1000 | UIF_NO_XOR |
  | 2 | 0xc00 | UBLINEAR_2 |
  | 3 | 0xb00 | UBLINEAR_1 |
  | 4 to 6 | 0xac0, 0xa80, 0xa40 | LINEARTILE |

  The whole image is 24 KiB, in a 32 KiB buffer. The CPU fills the buffer
  with the sentinel `0xa5` and then tiles level 0 into it, as
  `glTexImage2D` does before `glGenerateMipmap`.
- **T2**: one TFU job, packed by `v3d_tfu71_pack` (Mesa's `v3dX(tfu)` for
  7.1):
  - In: the raster source (`ICFG` input format raster, data type RGBA8, `IIS`
    64 texels).
  - Out: the texture's level 0 (`IOA`), with `IOC` = DIMTW, NUMMM 6, output
    format UIF_NO_XOR and stride 8 UIF blocks. That is `0x00086061`, the
    value of Mesa's own job in `tools/v3d/pi5/gl_mipgen.stdout`. Mesa's
    source is the UIF level 0 in place, so its `ICFG` and `IIS` differ from
    ours.

  The registers are written in the order of
  `driver/docs/rpi5-v3d71-facts.md` section 7 (Linux's `v3d_tfu_job_run`):
  `IIA`, `IIS`, `ICA`, `IUA`, `IOA`, `IOC`, `IOS`, `COEF0..3`, and `ICFG`
  last, with its IOC bit (bit 0). The `ICFG` write starts the job. Before
  that write, the probe reads back `IIA`, `IIS`, `IOA`, `IOC` and `IOS`.

  The probe then polls the hub's `INT_STS` and `TFU_CS` every 10 us, for 1 s
  at most. The job is done on `TFUC` (hub bit 1), or when `CS`'s conversion
  counter has moved and `BUSY` is clear. A hub MMU bit ends the wait as a
  fault.
- **T3**: the texture is invalidated in the CPU's cache, then each level is
  untiled with the same layout (`v3d_tex_load_box`) and checked:
  - Level 0 must equal the pattern. DIMTW tells the TFU not to write the
    base level; if it writes it anyway, the texels must be the same.
  - Levels 1 to 6 must be within **2 per channel** of a 2x2 box filter,
    `(a+b+c+d+2)/4`, computed on the CPU from its own previous level (a side
    of 1 repeats its texel). The tolerance of 2 covers the TFU's rounding,
    or its building each level from its own previous one.
  - The bytes outside the levels (below level 6's offset, and past level
    0's end up to 32 KiB) must still be the sentinel.

  Each level's line also counts the pixels that still hold the sentinel,
  which shows a level the TFU did not write at all.

The caches are handled as in `rpi5_clear`. `CONFIG_CACHE_MANAGEMENT` is on,
the source and the texture are cleaned before the job, and the texture is
invalidated before the CPU reads it back. A cache call that fails ends the
run.

## The console

The probe prints `v3d_tfu: build ...`, `v3d_tfu: cache: ...` and
`v3d_tfu: rpi_5, ...`. Then it prints one line per step with its raw
values: `M1 mailbox`, `M1 power_on`, `M1 V3D clock`, three `M1 identity`
lines, `M1 TFU 7.1 block before the job: CS ... SU ...`, `M2 page table`,
`M2 mmu_init`, one `T1 level N` line per level, `T2 job` (every register
value), `T2 job: IOC equals|differs from Mesa's gl_mipgen 0x00086061`,
`T2 read back before ICFG`, `T2 wait: <TFUC|CS ...|a hub MMU fault|nothing> after N us:
hub INT_STS ... CS ... SU ...`, `T2 caches_clean`, `T3 level 0`, six
`T3 level N` lines and `T3 outside the levels`.

The last line is the verdict, which `tools/board.sh pi5` ends the run on.
It is printed exactly once on every path; `main()` ends through one
`verdict()`, which keeps a flag.

- `v3d_tfu: PASS (levels 0-6 checked, 0 px wrong: level 0 exact, levels 1-6
  (1365 px) within 2 of the 2x2 box (max M); done by TFUC in N us; IOC
  0x00086061; HUB_IDENT1 0x00081117 bit 17 0)`
- `v3d_tfu: FAIL (<reason>)`. The reasons that answer the question are:
  - `TFU registers read all-ones: CS ... SU ... before the job`
  - `TFU registers read 0|all-ones: the job's 5 read back so, not done in N
    us, CS ... SU ...` (no completion, and every register read back as 0, or
    every one as all-ones)
  - `timeout: no TFUC and no CS count in N us, CS ... SU ... hub INT_STS ...`
  - `MMU fault: hub INT_STS ... VIO_ADDR ... VIO_ID ... CS ... SU ...` (on 7.1
    the TFU's AXI ids are 0x7c and 0x7d, facts file section 8)
  - `level 0 wrong: N of 4096 px differ; mips ...`
  - `mips wrong: N of 1365 px of levels 1-6 off by more than 2 (first level
    L, max M, S still the sentinel); level 0 exact; ...`
  - `writes outside the levels: N bytes changed; levels 0-6 right; ...`

  The other reasons name the step that failed: `M1 mailbox`, `M1 power-on`,
  `M1 identity`, `TFU busy before the job`, `M2 page table`,
  `M2 cache clean`, `M2 MMU`, `T1 layout`, `T1 cache clean`,
  `T2 v3d_tfu71_pack refused the job`, `T3 cache invalidate`,
  `T3 level N untile refused`.

  Every reason after M1 ends with `HUB_IDENT1 0x...`.

Nothing runs afterwards. The board layer's PM watchdog
(`CONFIG_YAKOGL_RPI5_WDOG`, armed in the first `PRE_KERNEL_1` hook) resets
the SoC, and tryboot brings Linux back. A hang ends the same way, with the
last step line on the console.

## On the board

Not run yet. The board session is its own plan item (`boardtfu71`):

```sh
tools/board.sh pi5 run build/rpi5_tfu/zephyr/zephyr.bin <logs>
```

That session records the verdict here, in `tools/cts/RESULTS.md`'s Pi 5
section, and in `docs/notes/pre-study-rpi5.md` where it answers a question
there.

## On the board (2026-11-21)

Built from origin/main `5857cc88` (image sha256 `0e963ee6`), run once with
`tools/board.sh pi5 run`: `rc=0 verdict=done boot_verdict=complete
result=legacy:v3d_tfu_pass resets=0 back_after=222s`; `get_throttled`
`0x0` before and after; the stock boot files were restored (cleanup line in
`meta.txt`).

```
v3d_tfu: PASS (levels 0-6 checked, 0 px wrong: level 0 exact, levels 1-6 (1365 px) within 2 of the 2x2 box (max 0); done by TFUC in 10 us; IOC 0x00086061; HUB_IDENT1 0x00081117 bit 17 0)
```

Raw identity words: `HUB_IDENT0..3 0x42554856 0x00081117 0x00001900
0x00020a10`, `CTL_IDENT0..2 0x07443356 0x81001431 0xc0078101`,
`MMU_DEBUG_INFO 0x20804664`. Job: `ICFG 0x00040000 IIA 0x00100000 IIS
0x00000040 IOC 0x00086061 IOA 0x00106000 IOS 0x00400040`, `TFUC` set
(`CVTCT 0 -> 1`). So the TFU works on 7.1.10 although its identity says
"TFU: no"; levels 1-6 matched the box filter exactly (max 0). Level 0
exact, 0 of 10816 bytes outside the levels changed.

## Limits

- One format (RGBA8) and one size (64x64), from a raster source. Mesa's
  in-place UIF source (`ICFG` input format 6) is not tried.
- `ICA`, `IUA` and `COEF0..3` are 0, as Mesa's 7.1 fill leaves them.
- The TFU is not reset after a timeout or a fault (`TFU_CS.TFURST` is never
  written); the watchdog's reset follows.
- `get_throttled` is recorded by `tools/board.sh` in `meta.txt`.
