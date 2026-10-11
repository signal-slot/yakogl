<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

> Record landed from `ccfarm/integration` (merge `e3efeaadc`, ccfarm `01a1284e-b46a-760f-bd69-23e1d4f8d33e`). Written on 2026-10-11 on the integration branch, not on main; the text is as written. Only file paths are changed to main's layout. A file this record names that is not on main stayed on the integration branch or was removed from main; check it with git before relying on it.

# Zephyr build environment for the gate rows

`tools/check.sh` builds four Zephyr images (AM62P gl_test, AM62P
precompiled-only, i.MX8MP gc_gl_test, rpi4 v3d_gl_test, all with
`-DYAKOGL_WERROR=ON`) and regenerates `driver/rust/src/soc/imx8mp/regs.rs`
from hal_nxp. Those rows need a west workspace, the Zephyr SDK and a
hal_nxp checkout. This document records the layout, the exact commands and
the versions on the Lepton worker node (ccfarm's name for this machine;
the user's own checkout is on nuc), so the same environment can be
rebuilt elsewhere.

The workspace, the SDK and the venv are node-local values registered in
ccfarm's project environment: `ZEPHYR_WS`, `ZEPHYR_SDK_INSTALL_DIR`,
`WEST_VENV` and `HAL_NXP_DIR`. `check.sh` reads `ZEPHYR_WS` and
`HAL_NXP_DIR` from the environment; without them the rows are gaps. No
machine path is committed into any script.

## Layout

The deps cache root is `$CCFARM_DEPS_DIR`
(`$CCFARM_DEPS_DIR` on Lepton):

```
zephyr-ws/                    # ZEPHYR_WS: the west workspace
  .west/config                # manifest path = yakogl (the clone below)
  zephyr/                     # pinned af17c0c8aa2a82a0b714a423915ad3610063ee68
  yakogl/                     # the manifest clone (see "Module list" below)
  modules/fs/fatfs            # pinned f4ead3bf4a6dab3a07d7b5f5315795c073db568d
  modules/hal/nxp             # pinned 1e91007b164c52b74d58d4e75a856efbc5d4de05 (HAL_NXP_DIR)
  modules/hal/ti              # pinned d9d5b1c9a72b21f9fc30f5815be2232b081e650c
  modules/lib/picolibc        # pinned 01254932e8e81085817ed61fd858648584ffe37c
  .venv/                      # WEST_VENV: west + zephyr/scripts/requirements-base.txt
zephyr-sdk-1.0.1/             # ZEPHYR_SDK_INSTALL_DIR (sdk_version: 1.0.1)
inputs/rgx.fw.36.53.104.796   # the board's PowerVR firmware (229,376 bytes)
dl/                           # the pinned tarballs the trees were proven against
deqp-stage1-rpi_4b/zephyr-ws            -> ../zephyr-ws
deqp-stage1-rpi_4b/sdk/zephyr-sdk-1.0.1 -> ../../zephyr-sdk-1.0.1
```

The two symlinks keep the paths baked into the existing stage-1 CTS
artifacts working after the workspace and the SDK moved to the cache root.

## Versions (measured on Lepton, 2026-10-08)

| Component | Version |
|---|---|
| west | 1.5.0 |
| venv Python | 3.14.8 |
| CMake (host) | 4.3.5 |
| ccache (host) | 4.14 |
| dtc (host) | 1.7.2 |
| Zephyr SDK | 1.0.1, GCC 14.3.0 (hosttools standalone 0.10 script present, not run) |
| Zephyr tree | 4.4.99 at `af17c0c8aa2a82a0b714a423915ad3610063ee68` |
| hal_nxp | `1e91007b164c52b74d58d4e75a856efbc5d4de05` |
| fatfs / hal_ti / picolibc | `f4ead3bf…` / `d9d5b1c9…` / `01254932…` (the zephyr manifest's pins) |
| cargo / rustc | rustup at `~/.cargo/bin`: cargo 1.98.0-nightly (a335d47ff 2026-06-26), rustc 1.98.0-nightly (4c9d2bfe4 2026-07-01) |

## Provisioning

The flow is `driver/samples/zephyr/README.md`'s:

```sh
python3 -m venv .venv && . .venv/bin/activate && pip install west
west init -m https://github.com/signal-slot/yakogl --mr main yakogl-ws
cd yakogl-ws && west update
pip install -r zephyr/scripts/requirements-base.txt
# SDK 1.0.1: https://docs.zephyrproject.org/latest/develop/toolchains/zephyr_sdk.html
#   zephyr-sdk-1.0.1_linux-x86_64_minimal.tar.xz  (plus the hosttools script)
```

This node's trees were assembled from codeload tarballs of the pinned
revisions instead of `west update`. Each tree was proven against its pin:
the pinned tarball was downloaded and `diff -r` compared with the tree
(zephyr: identical except the `.git` directory). The tarballs are kept in
`dl/`.

Two facts about that import, both checked later against the real commit
(`origin` = zephyrproject-rtos/zephyr, `git fetch --depth 1 origin
af17c0c8…` in `zephyr-ws/zephyr`):

- GitHub's codeload archive omits export-ignored files: zephyr's
  `.gitattributes` marks `.gitignore`, `.gitattributes` and `.mailmap`
  `export-ignore`, so the checkout has none of them. They are VCS
  metadata, not build inputs.
- The snapshot commit left four `build`-named directories untracked
  (`doc/build`, `scripts/build`, `scripts/tests/build`,
  `share/sysbuild/build`) because this node's global gitignore
  (`~/.gitignore`, `core.excludesFile`) has a `build` pattern — see
  "Node quirks". All four are present on disk (they are build inputs; the
  green builds prove it).

The checkout has since been canonicalized with `git checkout FETCH_HEAD`:
HEAD is the real `af17c0c8aa2a82a0b714a423915ad3610063ee68`, the metadata
files are on disk, and the four directories are tracked. All four image
rows were re-run from that state and are green with the same sizes
(2,431,248 / 5,089,224 / 1,846,120 / 1,988,992 bytes). The snapshot import
commit is gone.

## Gate rows: exact commands and results

`check.sh` runs these itself when `ZEPHYR_WS` is set (it exports
`ZEPHYR_BASE` and puts `$ZEPHYR_WS/.venv/bin` first on PATH). The manual
runs that produced the results below passed the toolchain as CMake cache
variables instead of exported environment variables — equivalent for
west/cmake — and `-DYAKOGL_CARGO` (see Node quirks). From the workspace
top (`WS` = ZEPHYR_WS, `SDK` = ZEPHYR_SDK_INSTALL_DIR, `REPO` = the
checkout):

```sh
cd "$WS"
mods=$("$REPO/tools/cts/zephyr-modules.sh" "$WS")   # with PATH="$WS/.venv/bin:$PATH"

# AM62P gl_test  -> zephyr.bin 2,431,248 B
.venv/bin/west build -p always -b verdin_am62p/am62p54/a53 \
    -d "$REPO/build/zephyr-werror/am62p" \
    "$REPO/driver/samples/zephyr/gl_test" -- \
    "-DZEPHYR_MODULES=$mods" "-DYAKOGL_DIR=$REPO" -DYAKOGL_WERROR=ON \
    -DCONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y \
    -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n \
    -DZEPHYR_TOOLCHAIN_VARIANT=zephyr -DZEPHYR_SDK_INSTALL_DIR="$SDK" \
    -DYAKOGL_CARGO="$HOME/.cargo/bin/cargo"

# AM62P without a compiler  -> zephyr.bin 5,089,224 B; same flags plus:
#   -DCONFIG_YAKOGL_SHADERC_PRECOMPILED_ONLY=y
#   '-DCONFIG_YAKOGL_PRECOMPILED_APPS="gl_test_es100"'
#   symbol checks on the ELF: shaderc11precompiled5alone15SHC_PRECOMPILED
#   present; no yakogl_shaderc_scratch, shc_online_ or 7shaderc6online.

# i.MX8MP gc_gl_test  -> zephyr.bin 1,846,120 B; board verdin_imx8mp/mimx8ml8/a53,
#   sample gc_gl_test, dir imx8mp, plus -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y

# rpi4 v3d_gl_test  -> zephyr.bin 1,988,992 B; board rpi_4b, sample v3d_gl_test

# hal_nxp registers
python3 "$REPO/tools/hal-nxp/gen_regs.py" "$WS/modules/hal/nxp" |
  cmp - "$REPO/driver/rust/src/soc/imx8mp/regs.rs"     # identical
```

All five rows are green on Lepton; the symbol checks ran with `nm` on the
built ELF. The build logs are in `build/zephyr-werror/*.log` in the
checkout.

The `YAKOGL_PRECOMPILED_APPS` value must keep its inner double quotes
through the shell (`'-DCONFIG_YAKOGL_PRECOMPILED_APPS="gl_test_es100"'`,
as `check.sh` writes it): without them Kconfig reports "malformed string
literal … Assignment ignored", and `-DYAKOGL_WERROR=ON` turns that warning
into a build failure.

## PowerVR firmware blob

The AM62P rows need `driver/src/fw/pvr_fw_blob.h` (gitignored, TI/Img
property, never committed). It is generated from the board's firmware file:

```sh
driver/tools/mkfwblob.sh "$CCFARM_DEPS_DIR/inputs/rgx.fw.36.53.104.796"
```

The script was run on Lepton and wrote the blob the builds used; re-running
it leaves the file unchanged (sha256
`ac0b4f14ebb6af44fef1e9999e2cca1ef07fef46f247bfa8d6744a73d8ac26d3`,
229,376 firmware bytes). `git check-ignore` confirms it is ignored
(`driver/.gitignore: src/fw/`).

## Node quirks

**Global gitignore.** `~/.gitignore` (this node's `core.excludesFile`)
ignores `build` and `builds` at any depth, plus `CLAUDE.md`, `.claude`
and a few editor files. A `git add` on this node therefore silently
skips every `build`-named path — that is what left zephyr's four
`build`-named source directories out of the snapshot commit above.
`git check-ignore -v <path>` names the rule.

**cargo on PATH.** The kache shim directory (`~/.local/lib/kache/shims`)
is first on this node's PATH and provides a `cargo` but no `rustc`. `capi/yakogl_capi.cmake`
requires a rustc next to the cargo it finds, so the gate's zephyr rows stop
with "no rustc next to …/kache/shims/cargo". Pick one:

- put `~/.cargo/bin` before the kache shims on PATH (it holds both cargo
  and rustc), or
- add a rustc proxy next to kache's cargo
  (`ln -s ~/.cargo/bin/rustc ~/.local/lib/kache/shims/rustc`), or
- pass `-DYAKOGL_CARGO=$HOME/.cargo/bin/cargo` (what the manual runs above
  did).

The rustup toolchain has the `aarch64-unknown-none` target installed; the
softfloat drivers archive is built with `-Z build-std=core` (the AUTO
setting in `yakogl_capi.cmake`).

## Module list

`tools/cts/zephyr-modules.sh` lists the workspace's projects and appends
the checkout's `driver/` and root last. The workspace's own `yakogl` clone
is listed too, but Zephyr keys modules by name and the last entry wins
(`zephyr_module.py`: `all_modules_by_name[meta['name']] = …`), so the
checkout's `yakogl` module is the one the build uses. A SafeUI checkout
that defines the same Verdin boards is left out by the script.

## Not verified

- The four images were built and inspected (sizes, symbols); none was run
  on a board.
- The SDK's hosttools script
  (`zephyr-sdk-1.0.1/hosttools/zephyr-sdk-x86_64-hosttools-standalone-0.10.sh`)
  is present but was not run: the builds used the host's cmake 4.3.5,
  ccache 4.14 and dtc 1.7.2, which satisfy Zephyr's minimums.
