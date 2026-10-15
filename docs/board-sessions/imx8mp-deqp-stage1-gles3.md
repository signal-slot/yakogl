<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP dEQP-GLES3 stage 1 and table image (item stage1-imx8mp, plan #19)

Date 2026-10-14, yakogl `ba8b242a` (origin/main `72ee696d` merged into the zephyr-env base). Result: **the gles3 stage 1
and the 133-row dEQP-GLES3 table image are built (rc=0). The `board.sh --dry-run` of the table is clean (133 rows,
all `rc=0`). Nothing was run on or sent to a board.** An earlier build of this item (2026-10-12, `83115f91`) exists in
git history; this one repeats it on this node and found a stage-1 bug (below). The stage 1 libraries are byte-identical
to that earlier build.

## Table check

```
$ python3 -I tools/cts/check-imx8mp-gles3-tree.py -q
mustpass 42810, entries 133 (max 256), longest entry 1868 chars (max 2035), largest entry 1000 cases, matched exactly once 42810
OK
$ python3 -I tools/cts/check-gles3-tree.py tools/cts/imx8mp-gles3-tree.txt | tail -1
check-gles3-tree: tools/cts/imx8mp-gles3-tree.txt: 133 images, 42810 matched once, 0 not matched, 0 matched more than once, 0 empty images
$ python3 -I tools/cts/imx8mp-gles3-gen.py | cmp - tools/cts/imx8mp-gles3-tree.txt     # identical
$ python3 -I tools/cts/check-deqp-images.py     # also checks the AM62P, Pi 4 and i.MX8MP GLES3 tables: OK
```

## Environment (`D` = `$CCFARM_DEPS_DIR`)

Values come from `tools/cts/deps-env.sh --check imx8mp` (all `ok`).

| Variable | Value |
|---|---|
| `ZEPHYR_WORKSPACE` / `ZEPHYR_VENV` / `ZEPHYR_SDK_INSTALL_DIR` / `HAL_NXP_DIR` | `D/zephyr-ws`, `D/zephyr-ws/.venv`, `D/zephyr-sdk-1.0.1`, `D/zephyr-ws/modules/hal/nxp` |
| `IMX8MP_OUT` | `D/imx8mp-out` (already set in the worker's environment; a link to `ccfarm-local/deps/yakogl/imx8mp-out`) |
| `DEQP_STAGE1` (gles3) | `D/imx8mp-out/deqp-stage1-gles3` (the gles2 one stays in `D/deqp-stage1-imx8mp`) |
| `CTS` | `D/VK_GL_CTS` (VK-GL-CTS `93bca01`; `cts/` of the stage 1 is a `--shared` clone) |

## Commands and results

```sh
tools/cts/imx8mp-deqp-stage1.sh gles3
tools/cts/imx8mp-deqp-build.sh --gles3 es3-ba8b242a 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt
tools/cts/imx8mp-gles3-board.sh --dry-run tree out-es3-dry
sh tools/cts/imx8mp-gles3-artefacts.sh es3-ba8b242a        # bytes and sha256 below
```

| step | rc | duration | output |
|---|---|---|---|
| stage 1 (gles3), first tries | 1 | about 1 min each | see "Stage-1 bug" |
| stage 1 (gles3), after the fix | 0 | 08:02:55 to 08:04:09 (1 min 14 s; the cached `gles3` tree already held most objects, so this is not a cold build; the 2026-10-12 cold build took about 14 min 50 s, 453 ninja steps) | `D/imx8mp-out/deqp-stage1-gles3/build`, log `D/imx8mp-out/deqp-stage1.log` |
| table image | 0 | 56 s (08:04:21 to 08:05:17) | `D/imx8mp-out/es3-ba8b242a/zephyr/zephyr.bin`, log `D/imx8mp-out/es3-ba8b242a.log` |

Image build output:

```
imx8mp: 133 table rows -> D/imx8mp-out/es3-ba8b242a.table.conf
rc=0
check-deqp-images: the dEQP image recipes build what conformance needs
```

`.config` of the image: `# CONFIG_DEQP_REPORT_ES2 is not set` and `# CONFIG_YAKOGL_REPORT_ES2 is not set`. ES 2.0
stays the default: only `--gles3` sets those two off (and `-DDEQP_MODULE=gles3`); no gles2 path was changed. No ES2
image was built in this item.

## Stage-1 bug found and fixed

The cached `deqp-stage1-gles3` directory came from another worker. Its `build/CMakeCache.txt` held that worker's
`-I` paths (a workspace that no longer exists), because CMake reads `CMAKE_<LANG>_FLAGS_INIT` of the toolchain file only
when it creates the cache. `cts-build-zephyr.sh` rewrote the toolchain file but not the cache, so ninja stopped at
`glwInitES20Direct.cpp.obj` with `fatal error: GLES2/gl2.h: No such file or directory`. The log showed it only far
above the `tail -5` that `imx8mp-deqp-stage1.sh` prints.

Fixes (this branch):
- `tools/cts/cts-build-zephyr.sh`: if `CMakeCache.txt` does not contain the current flags, drop it (ninja rebuilds only
  what the new flags touch).
- `tools/cts/imx8mp-deqp-stage1.sh`: on failure, print the first `FAILED:` step with its error lines and the include
  flags of its command.
- New `tools/cts/imx8mp-gles3-artefacts.sh`: bytes and sha256 of the artefacts below.

## Artefacts

| file (under `D/imx8mp-out/`) | bytes | sha256 |
|---|---|---|
| `es3-ba8b242a/zephyr/zephyr.bin` | 15,878,520 | `8df88235dddb22b386bd18d3bcde201afc6daba057a5eaed3e7c92a3b9bc8bd2` |
| `es3-ba8b242a/zephyr/zephyr.elf` | 37,299,424 | `2a4d2d68c1239857b6debe98b54ad85f3014fb3063808e023cd860b180c068b5` |
| `es3-ba8b242a.table.conf` | 38,164 | `5b7a612379058051e8be1d1983829e7c13254e817289282fe6e38f99bbedb82f` |
| `deqp-stage1-gles3/build/modules/gles3/libdeqp-gles3-package.a` | 132,960 | `7f2b1adda378dd3782733958524a59c5a95fe0cf8a02caece9db91f715e2e74b` |
| `deqp-stage1-gles3/build/modules/gles3/functional/libdeqp-gles3-functional.a` | 12,733,388 | `52168131db0f068f821e5f4560d73cafd702c03d49157066f9dbbeaa74388511` |

Image size: `zephyr.bin` 15,878,520 bytes; footprint checked by `board.sh`: 227,311,616 bytes (inside the board's RAM window).
The `.table.conf` and both libraries are byte-identical to the 2026-10-12 build; `zephyr.bin` differs because it embeds
the build hash.

## `board.sh --dry-run` of the table

`tools/cts/imx8mp-gles3-board.sh --dry-run tree out-es3-dry` (default boot backend `uboot-tftp`, `BOARD_BOOT` not set;
the recipe's `uboot-mmc` differs only in the staging steps): 133 rows `e00` .. `e132`, each ends `rc=0`
(`grep -c '^=== e[0-9]* rc=0'` gives 133), no board command ran. Row 0, abridged:

```
DRY host$ mkdir -p out-es3-dry; write out-es3-dry/tree.txt (133 rows)
board.sh imx8mp: tree: 133 rows from tools/cts/imx8mp-gles3-tree.txt, build ba8b242a
=== e00 (zephyr.bin, selector 0) 2026-10-14 08:05:33
board.sh imx8mp: e00: build ba8b242a boot uboot-tftp (row) image sha256 8df88235dddb22b3 footprint 227311616
board.sh imx8mp: e00: image build: PVR_SAMPLE_TABLE=c95547a1 PVR_SAMPLE_TREE=ba8b242a; zephyr: af17c0c8aa2a
DRY board$ printf <the 16 bytes '00000000 53450000 00000000 00000000'> | sudo -n tee /sys/bus/nvmem/devices/30370000.snvs:snvs-lpgpr0/nvmem; ...
DRY host$ cp .../imx8mp-out/es3-ba8b242a/zephyr/zephyr.bin /TFTP_ROOT/yakogl/e00-8df88235dddb22b3-<token>.bin
...
```

The image carries `PVR_SAMPLE_TREE=ba8b242a`. Rebuild the image at the commit the board session runs (name it
`es3-<hash8>`); the recipe has the exact `tools/board.sh` lines.

## Not done / for board-imx8mp

- Not booted. The 600 s watchdog sizing of the entries is an estimate (recipe, "Sizing").
- Failing names print as `crc:<hex>` until `board.sh` resolves them against the GLES3 case list (recipe, "GLES3 caveat").
- `tools/cts/am62p-deqp-images.sh` still passes only `-DCONFIG_DEQP_REPORT_ES2=n` for its GLES3 images; it also needs
  `-DCONFIG_YAKOGL_REPORT_ES2=n`. Not touched here.
- `tools/check.sh --changed 72ee696`: 25 ok, 0 failed, 0 gaps (includes the `board.sh` dry-run matrix); the full run was not made. `check-deqp-images.py` (its image-recipe row) passes.
