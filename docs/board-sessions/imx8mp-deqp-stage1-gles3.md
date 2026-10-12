# i.MX8MP dEQP-GLES3 stage 1 and table image (item stage1-imx8mp)

Date 2026-10-12, worker system76, yakogl `c734523d` (branch `ccfarm/01a12ebb-...`, on the plan's integration
tip). Result: **the gles3 stage 1 and the 133-row dEQP-GLES3 table image are built (rc=0, conformance
check passed). The `board.sh --dry-run` of the table is clean. Nothing was run on or sent to a board.**

## Table check

```
$ python3 -I tools/cts/check-imx8mp-gles3-tree.py -q
mustpass 42810, entries 133 (max 256), longest entry 1868 chars (max 2035), largest entry 1000 cases, matched exactly once 42810
OK
$ python3 -I tools/cts/imx8mp-gles3-gen.py | cmp - tools/cts/imx8mp-gles3-tree.txt     # identical
```

## Environment (`D` = `$CCFARM_DEPS_DIR` = `/home/tasuku/.local/share/ccfarm/deps/yakogl`)

| Variable | Value |
|---|---|
| `ZEPHYR_WORKSPACE` / `ZEPHYR_VENV` / `ZEPHYR_SDK_INSTALL_DIR` / `HAL_NXP_DIR` | `D/zephyr-ws`, `D/zephyr-ws/.venv`, `D/zephyr-sdk-1.0.1`, `D/zephyr-ws/modules/hal/nxp` |
| `IMX8MP_OUT` | `D/imx8mp-out` |
| `DEQP_STAGE1` (gles3) | `D/imx8mp-out/deqp-stage1-gles3` (the gles2 one stays in `D/deqp-stage1-imx8mp`) |
| CTS | VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` (`cts/` is a `--shared` clone of `D/VK-GL-CTS`) |

## Script changes made here

- `imx8mp-deqp-stage1.sh`: argument `gles3` (module gles3, directory `$DEQP_STAGE1_GLES3` or
  `$IMX8MP_OUT/deqp-stage1-gles3`); exports `ZEPHYR_SDK` from `ZEPHYR_SDK_INSTALL_DIR` and defaults
  `CFLAGS`/`CXXFLAGS` to `-O2` (the earlier i.MX8MP stage-1 session had to do both by hand; libpng's NEON
  code fails at -O0). The gles2 behaviour with no argument is as before.
- `imx8mp-deqp-build.sh`: leading `--gles3` (= `DEQP_MODULE=gles3` plus that stage 1 directory).
- `imx8mp-gles3-board.sh` (new): `board.sh imx8mp tree|tally` with the table and the newest `es3-*` image
  filled in.

## Commands and results

```sh
tools/cts/imx8mp-deqp-stage1.sh gles3
tools/cts/imx8mp-deqp-build.sh --gles3 es3-c734523d 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt
tools/cts/imx8mp-gles3-board.sh --dry-run tree /tmp/es3-dry
```

| step | rc | duration | output |
|---|---|---|---|
| stage 1 (gles3) | 0 | about 14 min 50 s (15:25:58 to 15:40:45, 453 ninja steps, cold) | `D/imx8mp-out/deqp-stage1-gles3/build`, log `D/imx8mp-out/deqp-stage1.log` (16,320 lines) |
| table image | 0 | about 2 min | `D/imx8mp-out/es3-c734523d/zephyr/zephyr.bin`, log `D/imx8mp-out/es3-c734523d.log` |

An earlier attempt (before the `gles3` argument existed) started with no `DEQP_MODULE` and was stopped at
once: it was rebuilding the gles2 stage 1 in place. Nothing in `D/deqp-stage1-imx8mp` was changed by it
beyond ninja's already-built objects.

Build output of the image:

```
imx8mp: 133 table rows -> D/imx8mp-out/es3-c734523d.table.conf
rc=0
check-deqp-images: the dEQP image recipes build what conformance needs
```

`.config` of the image: `# CONFIG_DEQP_REPORT_ES2 is not set`, `# CONFIG_YAKOGL_REPORT_ES2 is not set`,
`CONFIG_DEQP_CASE_TABLE="dEQP-GLES3.functional.attribute_location.*;..."`. The ES 2.0 images are not
affected: no gles2 path changed (`DEQP_MODULE` unset builds as before).

## Artefacts

| file | bytes | sha256 |
|---|---|---|
| `es3-c734523d/zephyr/zephyr.bin` | 15,788,048 | `b439fd2477167a2f0565f6cc43f6a2d74313ed1db2f7d923f3c858cde4399d2d` |
| `es3-c734523d/zephyr/zephyr.elf` | 36,183,760 | not taken |
| `es3-c734523d.table.conf` | | `5b7a612379058051e8be1d1983829e7c13254e817289282fe6e38f99bbedb82f` |
| `deqp-stage1-gles3/build/modules/gles3/libdeqp-gles3-package.a` | | `7f2b1adda378dd3782733958524a59c5a95fe0cf8a02caece9db91f715e2e74b` |
| `deqp-stage1-gles3/build/modules/gles3/functional/libdeqp-gles3-functional.a` | | `52168131db0f068f821e5f4560d73cafd702c03d49157066f9dbbeaa74388511` |

Image size: `zephyr.bin` 15,788,048 bytes; Zephyr's report `RAM: 221836 KB of 256 MB (84.62 %)`; footprint
checked by `board.sh`: 227,160,064 bytes (inside the board's RAM window). The gles2 table image was
221,282,304 bytes of footprint. Disk: stage 1 directory 2.0 GB.

## `board.sh --dry-run` of the table

`tools/cts/imx8mp-gles3-board.sh --dry-run tree /tmp/es3-dry` (default boot backend `uboot-tftp` because
`BOARD_BOOT` was not set; the recipe's `uboot-mmc` differs only in the staging steps): 133 rows, `e00` ..
`e132`, selectors 0..132, rc=0 for each, no board command run. Row 0, abridged:

```
DRY host$ mkdir -p /tmp/es3-dry; write /tmp/es3-dry/tree.txt (133 rows)
board.sh imx8mp: tree: 133 rows from tools/cts/imx8mp-gles3-tree.txt, build 635a3ae
=== e00 (zephyr.bin, selector 0) 2026-10-12 15:47:25
board.sh imx8mp: e00: build 635a3ae boot uboot-tftp (row) image sha256 b439fd2477167a2f footprint 227160064
board.sh imx8mp: e00: image build: PVR_SAMPLE_TABLE=c95547a1 PVR_SAMPLE_TREE=c734523; zephyr: v4.4.0-9848-gaf17c0c8aa2a
DRY board$ printf <the 16 bytes '00000000 53450000 00000000 00000000'> | sudo -n tee /sys/bus/nvmem/devices/30370000.snvs:snvs-lpgpr0/nvmem; ...
DRY host$ cp .../imx8mp-out/es3-c734523d/zephyr/zephyr.bin /TFTP_ROOT/yakogl/e00-b439fd2477167a2f-<t>.bin
DRY host$ sha256sum /TFTP_ROOT/yakogl/e00-b439fd2477167a2f-<t>.bin   # must be b439fd24...99d2d
DRY board$ sudo -n fw_setenv -s /dev/stdin <<'EOF' (yako_once, yako_src=net, yako_file, yako_server)
DRY board$ sync; sudo -n systemctl reboot
...
=== e00 rc=0 2026-10-12 15:47:25
```

`build 635a3ae` is the checkout's HEAD when `board.sh` ran; the image carries `PVR_SAMPLE_TREE=c734523`.
The board session should rebuild the image at the commit it runs (name it `es3-<hash8>`).

## Not done / for board-imx8mp

- Not booted. The 600 s watchdog sizing of the entries is an estimate (recipe, "Sizing").
- Failing names print as `crc:<hex>` until `board.sh` resolves them against the GLES3 case list.
- `tools/cts/am62p-deqp-images.sh` line 104 still passes only `-DCONFIG_DEQP_REPORT_ES2=n` (no
  `YAKOGL_REPORT_ES2=n`): its GLES3 images report ES 2.0. Not touched here (another item's file).
