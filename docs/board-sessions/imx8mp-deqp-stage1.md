# imx8mp-deqp-stage1b: i.MX8MP dEQP stage 1 (BLOCKED, nothing built)

Date: 2026-10-08. Result: no stage 1, no mini-table image. The first command
that needs the shared Zephyr workspace was refused, so I stopped without
workarounds. There is no rc, time or disk use to report. No board was touched
and nothing was run over ssh.

## State found (worker system76)

`$CCFARM_DEPS_DIR` = `/home/tasuku/.local/share/ccfarm/deps/yakogl`.

- `zephyr-ws/.venv/bin/west` exists (the venv now has west; the
  `zephyr-ws-setup` note still says it did not).
- `zephyr-ws/.west/config` exists and `zephyr-ws/yakogl/` holds only
  `west.yml`: `west init` was done, `west update` was not.
  `zephyr-ws/zephyr/` is empty.
- No Zephyr SDK (`zephyr-sdk-1.0.1`) in the deps dir.
- No `deqp-stage1-imx8mp`, `imx8mp-out` or `hal_nxp` yet (`west update` brings
  hal_nxp in with the allowlist of `west.yml`).
- CTS revision: not determined; `cts-build-zephyr.sh` fetches it and it is
  recorded in `$DEQP_STAGE1/cts` once stage 1 runs.

## Refused command

```
/home/tasuku/.local/share/ccfarm/deps/yakogl/zephyr-ws/.venv/bin/west --version
```

("This command requires approval.") A `cd .../zephyr-ws && ... west update`
compound was refused the same way. Every use of the venv's `west` is therefore
blocked here; `west update` is the next step.

## Update (second turn)

`./.venv/bin/west update` (run from `zephyr-ws`, allowed) fetched zephyr at
`af17c0c8aa2`, fatfs, hal_nxp (`1e91007b1`, `modules/hal/nxp`), hal_ti and
picolibc (`01254932e`). `modules/lang` (zephyr-lang-rust) is absent, so either
the update stopped after picolibc or the module is cloned elsewhere; unchecked.
Still refused: `./.venv/bin/west list` and
`./.venv/bin/python -m pip install -r zephyr/scripts/requirements-base.txt`
("This command requires approval"). No SDK exists, so stage 1 and the mini
table are still not built. Re-run `west update` first to see whether
zephyr-lang-rust completes.

## Update (third turn)

`west list` and the pip install were then allowed and ran. `west list`:
zephyr `af17c0c8aa2a82a0b714a423915ad3610063ee68`, fatfs `f4ead3bf`, hal_nxp
`1e91007b` (`modules/hal/nxp`), hal_ti `d9d5b1c9`, picolibc `01254932`; no
zephyr-lang-rust project is listed. `pip install -r
zephyr/scripts/requirements-base.txt`: every requirement already satisfied
(west 1.5.0, Python 3.14 venv). The workspace is complete except the SDK.

Refused: `curl -sIL https://github.com/zephyrproject-rtos/sdk-ng/releases/download/v1.0.1/zephyr-sdk-1.0.1_linux-x86_64_minimal.tar.xz`
("This command requires approval"). I did not fetch the SDK another way. With
no `ZEPHYR_SDK_INSTALL_DIR`, stage 1 and the mini table are still not built.

## Commands to run once permitted

```sh
D=/home/tasuku/.local/share/ccfarm/deps/yakogl
cat > $D/zephyr-ws.gitconfig <<'CFG'
[url "git@github.com:"]
	insteadOf = https://github.com/
CFG
export GIT_CONFIG_GLOBAL=$D/zephyr-ws.gitconfig
cd $D/zephyr-ws && $D/zephyr-ws/.venv/bin/west update
$D/zephyr-ws/.venv/bin/python -m pip install -r zephyr/scripts/requirements-base.txt
# SDK 1.0.1 minimal bundle into $D, ./setup.sh -t aarch64-zephyr-elf (no -c)

export ZEPHYR_WORKSPACE=$D/zephyr-ws ZEPHYR_VENV=$D/zephyr-ws/.venv
export ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1
export IMX8MP_OUT=$D/imx8mp-out DEQP_STAGE1=$D/deqp-stage1-imx8mp
export HAL_NXP_DIR=$D/zephyr-ws/modules/hal/nxp   # if the build asks for it
tools/cts/imx8mp-deqp-stage1.sh                   # about 1.9 GB

TBL=$IMX8MP_OUT/mini-table.txt
grep -vE '^[[:space:]]*(#|$)' tools/cts/imx8mp-full-tree.txt | sed -n '3p;22p' > $TBL
RUSTUP_TOOLCHAIN=nightly tools/cts/imx8mp-deqp-build.sh mini 'dEQP-GLES2.info.*' --table $TBL
```
