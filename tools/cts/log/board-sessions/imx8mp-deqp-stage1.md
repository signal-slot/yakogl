# imx8mp-deqp-stage1b: i.MX8MP dEQP stage 1 and the two-row mini-table image

Date: 2026-10-08, worker host A, yakogl `14d6c177` (integration tip). Result:
stage 1 built (rc=0) and the two-row mini-table image built with
`imx8mp-deqp-build.sh --table` (rc=0, image check passed). No board was touched
and nothing was run over ssh.

## Inputs (`D` = `<deps>`)

| Variable | Value |
|---|---|
| `ZEPHYR_WORKSPACE` (= `ZEPHYR_WS`) | `D/zephyr-ws`, zephyr `af17c0c8aa2a82a0b714a423915ad3610063ee68` (`west list`: fatfs `f4ead3bf`, hal_nxp `1e91007b`, hal_ti `d9d5b1c9`, picolibc `01254932`) |
| `ZEPHYR_VENV` (= `WEST_VENV`) | `D/zephyr-ws/.venv` (west 1.5.0, requirements-base satisfied) |
| `ZEPHYR_SDK_INSTALL_DIR` | `D/zephyr-sdk-1.0.1` (gcc 14.3.0 aarch64-zephyr-elf) |
| `HAL_NXP_DIR` | `D/zephyr-ws/modules/hal/nxp` (from `west update`; not needed beyond the workspace) |
| `DEQP_STAGE1` | `D/deqp-stage1-imx8mp` |
| `IMX8MP_OUT` | `D/imx8mp-out` |
| CTS revision | VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` (`DEQP_STAGE1/cts` is a `--shared` clone of `D/VK-GL-CTS`) |

## Commands and results

Two things the scripts did not do by themselves on this worker:

- `cts-build-zephyr.sh` reads `ZEPHYR_SDK` (default
  `~/org/zephyrproject/zephyr-sdk-1.0.1`), not `ZEPHYR_SDK_INSTALL_DIR`; the
  first run failed with "CMAKE_C_COMPILER is not a full path to an existing
  compiler tool". Fix: export `ZEPHYR_SDK` too.
- The libpng NEON file failed with `arm_neon.h: lane index must be a constant
  immediate`: the toolchain file passes no `-O` (`CMAKE_C_FLAGS_RELEASE` is
  empty), so the code is built at -O0. Fix without a repo change: `CFLAGS=-O2
  CXXFLAGS=-O2` in the environment, and a fresh `DEQP_STAGE1/build`. Whether
  the host D builds carry an `-O` from elsewhere is not known; a permanent fix
  would be an `-O2` in `cts-build-zephyr.sh`'s toolchain file (not done: out of
  this item).

```sh
D=<deps>
export CFLAGS=-O2 CXXFLAGS=-O2 ZEPHYR_WORKSPACE=$D/zephyr-ws ZEPHYR_VENV=$D/zephyr-ws/.venv
export ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1 ZEPHYR_SDK=$D/zephyr-sdk-1.0.1
export IMX8MP_OUT=$D/imx8mp-out DEQP_STAGE1=$D/deqp-stage1-imx8mp HAL_NXP_DIR=$D/zephyr-ws/modules/hal/nxp
time tools/cts/imx8mp-deqp-stage1.sh        # rc=0, real 7m30s (user 22m40s); log $IMX8MP_OUT/deqp-stage1.log
                                            # (an earlier part of the ninja build had run before: 417 steps in total)
TBL=$IMX8MP_OUT/mini-table.txt
grep -vE '^[[:space:]]*(#|$)' tools/cts/imx8mp-full-tree.txt | sed -n '3p;22p' > $TBL
RUSTUP_TOOLCHAIN=nightly time tools/cts/imx8mp-deqp-build.sh mini 'dEQP-GLES2.info.*' --table $TBL
```

Mini table (row 1: the shaders tail incl. `shaders.fragdata.*`; row 2:
`dEQP-GLES2.functional.fbo.*`).

Image build output (`real 0m53s`):

```
imx8mp: 2 table rows -> <deps>/imx8mp-out/mini.table.conf
rc=0
CONFIG_DEQP_REPORT_ES2=y
check-deqp-images: the dEQP image recipes build what conformance needs
```

Image: `IMX8MP_OUT/mini/zephyr/zephyr.bin`, 10,006,504 bytes.

## Disk use

`DEQP_STAGE1` 2.0 GB; `IMX8MP_OUT` 276 MB (stage 1 log, mini image).

## Not done

The image was not run (no board). The stage 1 is for this CTS revision only.
