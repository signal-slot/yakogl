<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Verdin iMX95 probes

Zephyr images for the Toradex Verdin iMX95 on the Mallow
(`verdin_imx95/mimx9596/a55`, i.MX 95, Mali-G310), one directory each, for
the first board session. Each answers one question and ends with one verdict
line; `examples/triangle` does everything at once and answers none of them
alone. Run them in this order: each assumes the facts the ones before it
established. The plans they serve are `docs/notes/pre-study-imx95.md`
(section 6, B1 to B3; section 7, U1 to U6) and `driver/docs/imx95-display.md`
(section 6, M1 to M4 and Z0 to Z2; section 7, U1 to U21).

| Probe | Question | Verdict line | Settles |
|---|---|---|---|
| `linux_day_one/` (not an image) | what the module's own Linux shows: board, kernel, memory, the display it drives, the bridge's registers, the SM's clocks and domains, the golden registers | the record's `# end`; no verdict | B1 (Linux half), M1 to M4; pre-study U1 (the SM's version, if Linux shows it); display note U3, U8, U10, U13, U16, U18, U20 |
| `imx95_wdog` | does WDOG3 reset the board, and what did U-Boot leave in it? | `imx95_wdog: not fed: reset expected within 60000 ms ...`, then U-Boot's banner; `imx95_wdog: FAIL no reset ...` if it did not | the watchdog (pre-study section 1.6): the reset every later probe relies on |
| `imx95_scmi` | what the System Manager tells AP-NS, read only | `imx95_scmi: PASS N facts, ...` / `FAIL`; `imx95_scmi/check.sh` against `expected.txt` | B2 (SCMI half), Z0; pre-study U1 (the agent); display note U2 |
| `imx95_ident` | what the GPU is, after the port's own power-on over SCMI | `imx95_ident: PASS GPU_ID 0x..., SHADER_PRESENT 0x..., N of 9 mock.rs words differ` / `FAIL` | B2 (GPU half); pre-study U5, U6; `gpu/mali/mock.rs` `G310_ASSUMED` |
| `imx95_fw` | does the CSF firmware boot from our side, and what does it answer | `imx95_fw: PASS GLB 1.5.0, N groups, CS features 0x..., suspend A/B` / `FAIL step N (...)` | B3 |
| `imx95_dsi_ids` | is the X4 path there: DSI host, PHY CSR, expander, bridge | `imx95_dsi_ids: PASS dsi version 0x..., expander answers, bridge SN65DSI83 after N ms` / `FAIL` | display note U13, U16, U18, U19 (the expander's answer), U21 |

## Rules every Verdin iMX95 probe follows

- **WDOG3 is armed first.** The board glue
  (`driver/zephyr/lib/board/verdin_imx95/verdin_imx95_wdog.c`) arms it at
  `PRE_KERNEL_1` in every image, 60 s, and feeds it up to the lifetime
  (`CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S`, 60 s); its timeout is the System
  Manager's whole-board reset. Every probe prints, first,
  `<probe>: wdog found CS ... TOVAL ... (running|stopped); armed rc 0 CS ...
  TOVAL ... timeout 60000 ms` (`common/vi95_probe.c`). `imx95_wdog` alone
  is built with the feeding off (`CONFIG_YAKOGL_VERDIN_IMX95_WDOG_FEED=n`).
- **Every run ends in the PSCI reset** (`verdin_imx95_reset`, TF-A's
  SYSTEM_RESET; U-Boot and Linux follow), after its verdict line. If the
  reset returns, WDOG3 resets the board within 60 s.
- **Lines.** Every line a probe prints starts with `<probe>: `. Facts a
  script reads are `<probe>: <key> = <value>`; the verdict is `<probe>: PASS
  ...` or `<probe>: FAIL <why>`, once, before the reset.
- **Build** from a west workspace, with `-DYAKOGL_WERROR=ON`; `imx95_fw`
  needs `driver/src/fw/mali_csffw_blob.h` (`driver/tools/mkmalifw.sh` from
  the pinned `MALI_CSFFW`; never committed):

  ```sh
  export $(tools/cts/deps-env.sh)
  bash driver/tools/mkmalifw.sh "$MALI_CSFFW" driver/src/fw/mali_csffw_blob.h
  west build -p always -b verdin_imx95/mimx9596/a55 -DYAKOGL_WERROR=ON -d build/<probe> yakogl/tools/probes/verdin_imx95/<probe>
  ```

  The gate's row `zephyr  verdin_imx95 probe images` builds every directory
  here that has a `CMakeLists.txt` and a `prj.conf` (a gap without the west
  workspace or the firmware).
- **Run** through the U-Boot route of `docs/standalone-boot.md` ("Toradex
  Verdin iMX95"), the image loaded at 0xd0000000. The board, its console and
  its ssh destination come from the environment (the farm's leases), never
  from a file. Name no address, host or UART path in a file or a commit.
- **If the board does not come back** within the runner's limit: stop; do
  not retry; report that the board needs a person.

## Common code

`common/vi95_probe.{h,c,cmake}`: the banner (build and WDOG3 line), one raw
SCMI message over the board's channel (`verdin_imx95_scmi`), the status
names, the end. The Rust side (the drivers crate) holds what is more than a
register read: `yd_imx95_scmi_survey` (`soc::imx95::survey`, read-only
messages only, host-tested against the mock SM), `yd_imx95_gpu_power_on` /
`_off` and `yd_imx95_fw_up` (`ffi::imx95`, `board::imx95_gpu`).
