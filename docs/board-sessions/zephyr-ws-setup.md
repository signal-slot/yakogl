# zephyr-ws-setup: shared Zephyr west workspace and SDK (BLOCKED, nothing built)

Date: 2026-10-09 (second attempt, worker XPS139340; first attempt 2026-10-08, worker system76). Result: no workspace, venv with west, or SDK exists on this
worker, and none could be created: the `pip install west` below is still
refused. No sample image was built, so there is no rc=0, time or disk use.

## State of this worker (system76)

- `$CCFARM_DEPS_DIR` = `/home/tasuku/.local/share/ccfarm/deps/yakogl`
  (contains `deqp-stage1-rpi_4b`, `mesa-26.2.1-src`, `mesa-hwdb`, `zephyr-ws`).
- `zephyr-ws/` holds only an empty `.venv` (no west, no Zephyr tree, no `.west`).
- No Zephyr SDK in the deps dir.
- `ask_farm` (allowed on the second turn) answered from nuc (ccfarm env and
  HANDOFF, confidence 0.98): `ZEPHYR_WS=/home/tasuku/com/github/signal-slot/zephyr-ws-yakogl`,
  `WEST_VENV=$ZEPHYR_WS/.venv`,
  `ZEPHYR_SDK_INSTALL_DIR=/home/tasuku/org/zephyrproject/zephyr-sdk-1.0.1`.
  Reading `sdk_version`, `.west/config` and `.venv/pyvenv.cfg` there gave "File
  does not exist" on this worker: those paths are node-local to nuc and
  unusable here. (Node Lepton named another tree, `~/org/zephyrproject/zephyrproject`,
  Zephyr 4.4.1, which is not the repo's pin.)
- SSH to GitHub works: `git ls-remote git@github.com:zephyrproject-rtos/zephyr.git HEAD`
  returned a hash.

## Pin

The repository root `west.yml` is the manifest (`self.path: yakogl`). It pins
zephyr `af17c0c8aa2a82a0b714a423915ad3610063ee68` with `name-allowlist`
fatfs, hal_nxp, hal_ti, picolibc, zephyr-lang-rust, and uses the remote
`url-base: https://github.com/zephyrproject-rtos` (HTTPS).
`driver/samples/zephyr/README.md` documents the setup.

## Still-refused command

2026-10-09, worker XPS139340: refused twice in a row, as two separate simple commands
("This command requires approval"). The deps dir on this worker still holds only
an empty `zephyr-ws/.venv` (plus `sdk-dl/`, empty). Command:

`/home/tasuku/.local/share/ccfarm/deps/yakogl/zephyr-ws/.venv/bin/python -m pip install west`
("This command requires approval", asked on every retry). Nothing after it was
attempted.

## Commands to run once it is permitted

HTTPS clone failed on a credential error before, so the manifest's HTTPS URLs
are rewritten to SSH by a git config file kept in the deps dir (not in `$HOME`).

```sh
D=$CCFARM_DEPS_DIR
cat > $D/zephyr-ws.gitconfig <<'CFG'
[url "git@github.com:"]
	insteadOf = https://github.com/
CFG
export GIT_CONFIG_GLOBAL=$D/zephyr-ws.gitconfig   # affects git run by west only in this shell

$D/zephyr-ws/.venv/bin/python -m pip install west
cd $D
$D/zephyr-ws/.venv/bin/west init -m git@github.com:signal-slot/yakogl.git --mr main zephyr-ws
#   the manifest repo lands in zephyr-ws/yakogl (west.yml: self.path)
cd zephyr-ws && $D/zephyr-ws/.venv/bin/west update
$D/zephyr-ws/.venv/bin/python -m pip install -r zephyr/scripts/requirements-base.txt
# SDK 1.0.1, x86_64 linux, aarch64 toolchain only -> $D/zephyr-sdk-1.0.1
#   download the zephyr-sdk-1.0.1_linux-x86_64 minimal bundle from the Zephyr SDK
#   releases, extract it into $D, run ./setup.sh -t aarch64-zephyr-elf. setup.sh can
#   register the SDK in ~/.cmake/packages (outside the deps dir): do not pass its
#   -c option; ZEPHYR_SDK_INSTALL_DIR is enough.
export ZEPHYR_WS=$D/zephyr-ws ZEPHYR_WORKSPACE=$D/zephyr-ws
export WEST_VENV=$D/zephyr-ws/.venv ZEPHYR_VENV=$D/zephyr-ws/.venv
export ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1
```

`requirements-base.txt` is what the samples README installs; the full
`requirements.txt` is not needed for these builds. Check `west list` shows
zephyr at `af17c0c8` before using it. Then prove the setup with
`tools/cts/common-images.sh <board> tools/cts/common-tree.txt $TMPDIR/img GT`.
