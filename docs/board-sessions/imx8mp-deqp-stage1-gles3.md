# i.MX8MP dEQP-GLES3 stage 1 and table image (item stage1-imx8mp)

Date 2026-10-12, yakogl `83115f91` (on the plan's integration tip `236f046`). Result: **the gles3 stage 1
and the 133-row dEQP-GLES3 table image are built (rc=0, conformance check passed). The `board.sh --dry-run`
of the table is clean. Nothing was run on or sent to a board.** This is the second build of this item (the
first pull request conflicted with main); the stage 1 libraries are byte-identical to the first build.

## Table check

```
$ python3 -I tools/cts/check-imx8mp-gles3-tree.py -q
mustpass 42810, entries 133 (max 256), longest entry 1868 chars (max 2035), largest entry 1000 cases, matched exactly once 42810
OK
$ python3 -I tools/cts/check-gles3-tree.py tools/cts/imx8mp-gles3-tree.txt | tail -1
check-gles3-tree: tools/cts/imx8mp-gles3-tree.txt: 133 images, 42810 matched once, 0 not matched, 0 matched more than once, 0 empty images
$ python3 -I tools/cts/imx8mp-gles3-gen.py | cmp - tools/cts/imx8mp-gles3-tree.txt     # identical
```

`tools/cts/check-deqp-images.py` (part of `tools/check.sh`) now also runs `check-gles3-tree.py` on this table.

## Environment (`D` = `$CCFARM_DEPS_DIR` = `/home/tasuku/.local/share/ccfarm/deps/yakogl`)

| Variable | Value |
|---|---|
| `ZEPHYR_WORKSPACE` / `ZEPHYR_VENV` / `ZEPHYR_SDK_INSTALL_DIR` / `HAL_NXP_DIR` | `D/zephyr-ws`, `D/zephyr-ws/.venv`, `D/zephyr-sdk-1.0.1`, `D/zephyr-ws/modules/hal/nxp` |
| `IMX8MP_OUT` | `D/imx8mp-out` (already set in the worker's environment) |
| `DEQP_STAGE1` (gles3) | `D/imx8mp-out/deqp-stage1-gles3` (the gles2 one stays in `D/deqp-stage1-imx8mp`) |
| `CTS` | `D/VK_GL_CTS`, VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` (`cts/` of the stage 1 is a `--shared` clone) |

## Scripts

- `imx8mp-deqp-stage1.sh`: argument `gles3` (module gles3, directory `$DEQP_STAGE1_GLES3` or
  `$IMX8MP_OUT/deqp-stage1-gles3`); exports `ZEPHYR_SDK` from `ZEPHYR_SDK_INSTALL_DIR` and defaults
  `CFLAGS`/`CXXFLAGS` to `-O2` (libpng's NEON code fails at -O0). No argument: the gles2 behaviour as before.
- `imx8mp-deqp-build.sh`: leading `--gles3` (= `DEQP_MODULE=gles3` plus that stage 1 directory). A gles3 image
  gets `-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n` and is checked with
  `check-deqp-images.py --config ... --gles3`. Without `--gles3` nothing changes: ES 2.0 stays the default.
- `imx8mp-gles3-board.sh`: `board.sh imx8mp tree|tally` with the table and the newest `es3-*` image filled in.

## Commands and results

```sh
tools/cts/imx8mp-deqp-stage1.sh gles3
tools/cts/imx8mp-deqp-build.sh --gles3 es3-83115f91 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt
tools/cts/imx8mp-gles3-board.sh --dry-run tree out-es3-dry
```

| step | rc | duration | output |
|---|---|---|---|
| stage 1 (gles3) | 0 | seconds here: the cached tree `D/imx8mp-out/deqp-stage1-gles3` was already complete, ninja only re-linked. The cold build of the first run was about 14 min 50 s (453 ninja steps) | `D/imx8mp-out/deqp-stage1-gles3/build`, log `D/imx8mp-out/deqp-stage1.log` |
| table image | 0 | 1 min 19 s (23:01:53 to 23:03:12) | `D/imx8mp-out/es3-83115f91/zephyr/zephyr.bin`, log `D/imx8mp-out/es3-83115f91.log` |

Build output of the image:

```
imx8mp: 133 table rows -> D/imx8mp-out/es3-83115f91.table.conf
rc=0
check-deqp-images: the dEQP image recipes build what conformance needs
```

`.config` of the image: `# CONFIG_DEQP_REPORT_ES2 is not set`, `# CONFIG_YAKOGL_REPORT_ES2 is not set`. The
ES 2.0 images are not affected: no gles2 path changed (`DEQP_MODULE` unset builds as before).

## Artefacts

| file (under `D/imx8mp-out/`) | bytes | sha256 |
|---|---|---|
| `es3-83115f91/zephyr/zephyr.bin` | 15,870,328 | `1f1e78810a2992593998526e088c20a95b6c1bcf2361a03ba8d77d0d01ac751d` |
| `es3-83115f91/zephyr/zephyr.elf` | 37,197,296 | `b2d9f1c0ac19e1682ed3c5da5d21e8999216e580ae71bd372b1056bfb62b1719` |
| `es3-83115f91.table.conf` | 38,164 | `5b7a612379058051e8be1d1983829e7c13254e817289282fe6e38f99bbedb82f` |
| `deqp-stage1-gles3/build/modules/gles3/libdeqp-gles3-package.a` | 132,960 | `7f2b1adda378dd3782733958524a59c5a95fe0cf8a02caece9db91f715e2e74b` |
| `deqp-stage1-gles3/build/modules/gles3/functional/libdeqp-gles3-functional.a` | 12,733,388 | `52168131db0f068f821e5f4560d73cafd702c03d49157066f9dbbeaa74388511` |

Image size: `zephyr.bin` 15,870,328 bytes; Zephyr's report `RAM: 221976 KB of 256 MB (84.68 %)`; footprint checked
by `board.sh`: 227,303,424 bytes (inside the board's RAM window).

## `board.sh --dry-run` of the table

`tools/cts/imx8mp-gles3-board.sh --dry-run tree out-es3-dry` (default boot backend `uboot-tftp` because
`BOARD_BOOT` was not set; the recipe's `uboot-mmc` differs only in the staging steps): 133 rows, `e00` ..
`e132`, selectors 0..132, rc=0 for each, no board command run. Row 0, abridged:

```
DRY host$ mkdir -p out-es3-dry; write out-es3-dry/tree.txt (133 rows)
board.sh imx8mp: tree: 133 rows from tools/cts/imx8mp-gles3-tree.txt, build 83115f9
=== e00 (zephyr.bin, selector 0) 2026-10-12 23:03:17
board.sh imx8mp: e00: build 83115f9 boot uboot-tftp (row) image sha256 1f1e78810a299259 footprint 227303424
board.sh imx8mp: e00: image build: PVR_SAMPLE_TABLE=c95547a1 PVR_SAMPLE_TREE=83115f9; zephyr: af17c0c8aa2a
DRY board$ printf <the 16 bytes '00000000 53450000 00000000 00000000'> | sudo -n tee /sys/bus/nvmem/devices/30370000.snvs:snvs-lpgpr0/nvmem; ...
DRY host$ cp .../imx8mp-out/es3-83115f91/zephyr/zephyr.bin /TFTP_ROOT/yakogl/e00-1f1e78810a299259-<token>.bin
DRY host$ sha256sum /TFTP_ROOT/yakogl/e00-1f1e78810a299259-<token>.bin   # must be 1f1e78810a299259...01ac751d
DRY board$ sudo -n fw_setenv -s /dev/stdin <<'EOF' (yako_once, yako_src=net, yako_file, yako_server)
DRY board$ sync; sudo -n systemctl reboot
...
=== e00 rc=0 2026-10-12 23:03:17
```

The image carries `PVR_SAMPLE_TREE=83115f9`. The board session should rebuild the image at the commit it
runs (name it `es3-<hash8>`).

## Not done / for board-imx8mp

- Not booted. The 600 s watchdog sizing of the entries is an estimate (recipe, "Sizing").
- Failing names print as `crc:<hex>` until `board.sh` resolves them against the GLES3 case list.
- `tools/cts/am62p-deqp-images.sh` still passes only `-DCONFIG_DEQP_REPORT_ES2=n` for its GLES3 images (check
  that file against the AM62P stage-1 doc); not touched here.
