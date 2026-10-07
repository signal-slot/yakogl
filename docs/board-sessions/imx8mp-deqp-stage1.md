# imx8mp-deqp-stage1b: i.MX8MP dEQP stage 1 (BLOCKED at the stage 1 command, nothing built)

Date: 2026-10-08. Result: no stage 1 and no mini-table image yet. The stage 1
command was refused, so there is no CTS revision, rc, time, disk use or image
check output. No board was touched and nothing was run over ssh.

## State (worker system76)

`D` = `$CCFARM_DEPS_DIR` = `/home/tasuku/.local/share/ccfarm/deps/yakogl`.

- Workspace `D/zephyr-ws` (`ZEPHYR_WORKSPACE`, venv `D/zephyr-ws/.venv` =
  `ZEPHYR_VENV`): `west update` done; `west list`: zephyr
  `af17c0c8aa2a82a0b714a423915ad3610063ee68`, fatfs `f4ead3bf`, hal_nxp
  `1e91007b` (`D/zephyr-ws/modules/hal/nxp` = `HAL_NXP_DIR`), hal_ti
  `d9d5b1c9`, picolibc `01254932`. No zephyr-lang-rust project is listed.
  `pip install -r zephyr/scripts/requirements-base.txt`: all satisfied
  (west 1.5.0, Python 3.14).
- SDK: `D/zephyr-sdk-1.0.1` exists (`sdk_version`, `gnu/aarch64-zephyr-elf`);
  it was installed by someone else during my session.
- `D/deqp-stage1-imx8mp` (`DEQP_STAGE1`) and `D/imx8mp-out` (`IMX8MP_OUT`) do
  not exist yet.

## Refused commands

```
ZEPHYR_WORKSPACE=D/zephyr-ws ZEPHYR_VENV=D/zephyr-ws/.venv ZEPHYR_SDK_INSTALL_DIR=D/zephyr-sdk-1.0.1 IMX8MP_OUT=D/imx8mp-out DEQP_STAGE1=D/deqp-stage1-imx8mp tools/cts/imx8mp-deqp-stage1.sh
```

(with `D` written out as the full path; "This command requires approval"), also
refused wrapped in `bash -c 'time ...'` with `HAL_NXP_DIR` and
`RUSTUP_TOOLCHAIN=nightly` added. Stopped there.

## Commands to run once permitted

```sh
D=/home/tasuku/.local/share/ccfarm/deps/yakogl
export ZEPHYR_WORKSPACE=$D/zephyr-ws ZEPHYR_VENV=$D/zephyr-ws/.venv
export ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1
export IMX8MP_OUT=$D/imx8mp-out DEQP_STAGE1=$D/deqp-stage1-imx8mp
export HAL_NXP_DIR=$D/zephyr-ws/modules/hal/nxp
time tools/cts/imx8mp-deqp-stage1.sh        # about 1.9 GB; log $IMX8MP_OUT/deqp-stage1.log

TBL=$IMX8MP_OUT/mini-table.txt
grep -vE '^[[:space:]]*(#|$)' tools/cts/imx8mp-full-tree.txt | sed -n '3p;22p' > $TBL
RUSTUP_TOOLCHAIN=nightly tools/cts/imx8mp-deqp-build.sh mini 'dEQP-GLES2.info.*' --table $TBL
du -sh $DEQP_STAGE1 $IMX8MP_OUT
```
