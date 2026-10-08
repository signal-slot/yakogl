# AM62P dEQP stage 1 (items am62p-deqp-stage1, stage1-am62p)

Date 2026-10-08, worker system76. Result: **stage 1 built for gles3 and gles2;
the deqp sample links for both with a stub firmware blob (link proof only); no
missing ES 3.0 symbol.** Nothing was run on or sent to a board.

## Environment

- `CCFARM_DEPS_DIR=/home/tasuku/.local/share/ccfarm/deps/yakogl`
- CTS: `$CCFARM_DEPS_DIR/VK-GL-CTS` at `93bca01861b0e3ef3c387027a9791e6d065f900c`
  (not written to; `cts-build-zephyr.sh` makes a `--shared` clone per module).
- `ZEPHYR_WS=$CCFARM_DEPS_DIR/zephyr-ws` (`ZEPHYR_BASE=$ZEPHYR_WS/zephyr`),
  `west` from `$ZEPHYR_WS/.venv/bin`, `ZEPHYR_SDK=ZEPHYR_SDK_INSTALL_DIR=$CCFARM_DEPS_DIR/zephyr-sdk-1.0.1`,
  `RUSTUP_TOOLCHAIN=nightly`, `BOARD=verdin_am62p/am62p54/a53`.
  `docs/es3-zephyr-env.md` was not on this branch.
- `ZEPHYR_MODULES="$(tools/cts/zephyr-modules.sh $ZEPHYR_WS)"` (the workspace's
  yakogl, fatfs, hal_nxp, hal_ti, picolibc, plus this checkout's `driver` and root).

## Stage 1 (commands as run)

```sh
cd <yakogl checkout>
for m in gles3 gles2; do
  MODULE=$m WORK=$CCFARM_DEPS_DIR/deqp-stage1-am62p-$m tools/cts/cts-build-zephyr.sh
done
```

| module | output | rc | duration | `build/` size | whole `WORK` |
|---|---|---|---|---|---|
| gles3 | `$CCFARM_DEPS_DIR/deqp-stage1-am62p-gles3/build` | 0 | 290 s (resumed: an earlier run was killed by a 10-minute tool limit after configure and part of the compile; the cold total is higher) | 106 M | 2.0 G |
| gles2 | `$CCFARM_DEPS_DIR/deqp-stage1-am62p-gles2/build` | 0 | 532 s (cold) | 94 M | 2.0 G |

`build/modules/<m>/libdeqp-<m>-package.a` (and functional/accuracy/performance/
stress) exist for each. Logs: `$CCFARM_DEPS_DIR/scratch/s1/<m>.log`.

## Link of the deqp sample

`driver/src/fw/pvr_fw_blob.h` is absent (not in the deps cache, never committed;
made by `driver/tools/mkfwblob.sh` from the board's `rgx.fw`). Without it
`pvr_fw_boot.c` fails to compile (`fw/pvr_fw_blob.h: No such file`), before any
link. So the link was proven with a 4-byte stub header placed outside the repo
(`$CCFARM_DEPS_DIR/scratch/s1/stubinc/fw/pvr_fw_blob.h`) and `-DEXTRA_CFLAGS=-I...`.
The resulting ELFs have no usable firmware and must not be booted.

```sh
west build -p always -b verdin_am62p/am62p54/a53 -d <builddir> driver/samples/zephyr/deqp -- \
  -DZEPHYR_MODULES="$ZEPHYR_MODULES" -DDEQP_SRC=$W/cts -DDEQP_BUILD=$W/build \
  -DDEQP_MODULE=<m> -DCONFIG_DEQP_REPORT_ES2=<y for gles2, n for gles3> \
  -DEXTRA_CFLAGS=-I$CCFARM_DEPS_DIR/scratch/s1/stubinc
```

(`-DDEQP_MODULE=gles3` and `CONFIG_DEQP_REPORT_ES2=n` are required: the sample
defaults to the gles2 package, and a first attempt without them failed with
`libdeqp-gles2-package.a ... missing` for the gles3 stage 1.)

| module | link rc | duration | `zephyr.elf` | undefined references |
|---|---|---|---|---|
| gles3 | 0 | 108 s | 40,034,848 B | 0 (every ES 3.0 entry point dEQP's table names resolves) |
| gles2 | 0 | 51 s | 32,618,120 B | 0 |

No case filter was passed; the link does not run cases.

## What is left

- Real AM62P images need the firmware blob (from `/lib/firmware/rgx.fw.36.53.104.796` on the board).
- The ELFs and build dirs under `$CCFARM_DEPS_DIR/scratch/s1/link-<m>` are stub-firmware proofs only.
