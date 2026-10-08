# ES2 host dEQP-GLES2 baseline (item es2-host-deqp-baseline)

Status: **NOT MEASURED.** The run did not happen. This file records why, and the exact
commands to run, so the item can be redone. It contains no measured number.

## Identification

- Integration commit: `1ad4695d64c07cf1ed8ab0d4dba4d34f668a400d`
- VK-GL-CTS checkout with the yakogl target: `/home/tasuku/.local/share/ccfarm/deps/yakogl/VK-GL-CTS`
  at `93bca01861b0e3ef3c387027a9791e6d065f900c` (the commit `docs/es3-host-env.md` pins;
  read from `.git/HEAD`), `targets/yakogl/yakogl.cmake` present. No build was needed or done.
- Back ends the script supports: `GPU=vivante` (GC7000, default) and `GPU=powervr`.
  There is no V3D switch (`host-deqp-run.sh` rejects any other `GPU`), so V3D cannot be measured
  on the host with this tool.

## What stopped the run

Every command that executes a program was refused in this unattended session with
"This command requires approval": `tools/cts/host-deqp-run.sh` (with and without `CTS=`/`GPU=`
prefixes, via `bash` too), `cmake --version`, `printenv`, `git -C <deps dir>`. Only file
listing and reading worked. The refusal is a permission denial, not a tool defect;
no output of the script exists.

## Commands to run (unchanged tool, HOST-ONLY results)

```sh
export CTS=/home/tasuku/.local/share/ccfarm/deps/yakogl/VK-GL-CTS
GPU=vivante tools/cts/host-deqp-run.sh    # -> build/host-deqp/run-<commit>
GPU=powervr tools/cts/host-deqp-run.sh    # -> build/host-deqp/run-<commit>-powervr
python3 tools/cts/cts-refusals.py --cases 2 <run dir>   # run again per back end if needed
```

The script itself prints cases begun against cases asked, the chunk exit statuses
(134 = aborted chunk) and the cts-refusals.py buckets. Default chunking (100) is kept.

## Template for the missing measurements (to fill in once run)

| Back end | asked | begun | aborted chunks (exit 134) | gaps | correct refusals |
|---|---|---|---|---|---|
| GC7000 (vivante), HOST-ONLY | – | – | – | – | – |
| PowerVR (powervr), HOST-ONLY | – | – | – | – | – |

Gap-bucket to plan-item map to apply to the result: preprocessor buckets (macro definition,
expansion order) → `es2-preprocessor`; ES 3-only enum/format accepted (negative_api) →
`es2-error-rules`; `shaders.invariance.*` → `es2-invariance`; no-table / offline-table
refusals → `es2-offline-table`; texture wrap/completeness → `es2-npot-cases`; any bucket
outside these is uncovered and must be flagged. Render verdicts from the host run are
meaningless (LINUX OSAL reads black) and must not be quoted.
