# zephyr-ws-setup: shared Zephyr west workspace and SDK (BLOCKED, nothing built)

Date: 2026-10-08. Result: no workspace, venv with west, or SDK could be set up;
the commands below were refused ("This command requires approval"). No sample
image was built, so there is no rc=0, no time and no disk use to report.

## What exists on this worker

- `$CCFARM_DEPS_DIR` = `/home/tasuku/.local/share/ccfarm/deps/yakogl`
  (contains `deqp-stage1-rpi_4b`, `mesa-26.2.1-src`, `mesa-hwdb`, `zephyr-ws`).
- `zephyr-ws/` holds only an empty `.venv` (no west, no Zephyr tree, no `.west`).
- No Zephyr SDK in the deps dir. The README's
  `$HOME/org/zephyrproject/zephyr-sdk-1.0.1` and
  `$HOME/com/github/signal-slot/zephyr-ws-yakogl` (Zephyr af17c0c8, per
  `tools/cts/README.md` lines 395-396) are outside the directories this
  session may list, so they could not be checked.
- `mcp__ccfarm__ask_farm` was refused (permission not granted), so the farm
  could not be asked where an existing workspace lives.
- The repository pins Zephyr only as commit `af17c0c8` (tools/cts/README.md);
  there is no west.yml in the repo.

## Update after the farm answered (second turn)

`ask_farm` was then allowed. Node nuc answered (sources: ccfarm env, ccfarm
HANDOFF), confidence 0.98:

- ZEPHYR_WS=/home/tasuku/com/github/signal-slot/zephyr-ws-yakogl
- WEST_VENV=/home/tasuku/com/github/signal-slot/zephyr-ws-yakogl/.venv
- ZEPHYR_SDK_INSTALL_DIR=/home/tasuku/org/zephyrproject/zephyr-sdk-1.0.1

(Node Lepton reported a different tree, ~/org/zephyrproject/zephyrproject, Zephyr
4.4.1, not the repo's pin.) These were NOT verified: `ls` and `Read` on those
paths are outside this session's allowed directories ("blocked" / permission
not granted), so `west --version`, `west list` and the SDK version file were
not run. `git ls-remote git@github.com:zephyrproject-rtos/zephyr.git HEAD`
now works (SSH to GitHub is reachable). `pip install west` was denied
("nobody decided in time"), so the deps-cache build was not attempted.

## Refused commands (exact)

1. `/home/tasuku/.local/share/ccfarm/deps/yakogl/zephyr-ws/.venv/bin/python -m pip install west`
2. `git ls-remote git@github.com:zephyrproject-rtos/zephyr.git HEAD` (SSH network probe)
3. tool call `mcp__ccfarm__ask_farm` (where do the Zephyr workspace/SDK live)

## Commands to run once these are permitted

```sh
D=$CCFARM_DEPS_DIR
$D/zephyr-ws/.venv/bin/python -m pip install west
cd $D/zephyr-ws
$D/zephyr-ws/.venv/bin/west init -m git@github.com:zephyrproject-rtos/zephyr.git --mr af17c0c8 .
$D/zephyr-ws/.venv/bin/west update
$D/zephyr-ws/.venv/bin/python -m pip install -r zephyr/scripts/requirements.txt
# SDK 1.0.1, x86_64 linux minimal bundle, then ./setup.sh -t aarch64-zephyr-elf,
# extracted to $D/zephyr-sdk-1.0.1
export ZEPHYR_WS=$D/zephyr-ws ZEPHYR_WORKSPACE=$D/zephyr-ws
export WEST_VENV=$D/zephyr-ws/.venv ZEPHYR_VENV=$D/zephyr-ws/.venv
export ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1
```

Then prove with
`tools/cts/common-images.sh <board> tools/cts/common-tree.txt $TMPDIR/img GT`.
