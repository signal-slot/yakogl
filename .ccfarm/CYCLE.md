# The yakogl development cycle, for ccfarm reviewers

## Cycle

**Where the truth lives.** The repository is `github.com/signal-slot/yakogl`
(`west.yml` names it as the manifest repository). `main` is the landing branch
and `origin/main` is the truth: `docs/HANDOFF.md` ("State of main") names the
commit of main the quoted results came from, and a board accepts only an image
built from a commit on `origin/main`. Work happens on short topic branches,
lately `work/<topic>` (earlier `<area>/<topic>`), rebased onto `origin/main`
before the gate. ccfarm merges agent results into `ccfarm/integration`; it does
not touch `main` until its review workspace has the environment below.

**How a change is gated.** There is no CI. `tools/check.sh` is the gate and
the project treats it as the whole pre-landing proof: a red `cargo fmt --check`
or a single warning is a defect. The full run (about 45 minutes on a warm
tree) is one script that, in order,

1. checks formatting and builds the `no_std` archive (`yakogl-capi`) that every
   product image links;
2. runs clippy with zero warnings and the test suite in every feature
   configuration and every feature pair the images are built from (PowerVR,
   GC7000, the ES 2.0 report, the in-process compiler, the precompiled tier,
   the compiler-less image, the test knobs, the V3D with its minimum test
   count and its ignored tests listed by name);
3. with the host C compiler, syntax-checks the FFI layout pins and the C the
   images compile, and runs the C check programs (V3D textures, tiling, TFU,
   PowerVR VDM/PPP fields, register words, DDK firmware-interface numbers,
   the Rust kernel side's records, the OSAL allocators), each of which must
   print its own "N checks run (planned N), 0 failed" line;
4. regenerates every generated file from its vendored inputs and requires the
   committed bytes (Mesa's packers and csbgen, the back ends' internal
   programs from our compiler, the bindgen output, the hal_nxp registers, the
   DDK core table), and checks `third_party/` against its `UPSTREAM` hashes;
5. compiles the product arms for `aarch64-unknown-none` without `host-test`,
   and counts symbols in the built archives: no shader table, no Mesa
   reference and no test knob in a product artefact, with a control build
   that must find them;
6. runs the Python tools' self-tests, the API coverage check with its pinned
   counts, `tools/board.sh`'s dry-run matrix and replays, and the host tools'
   own lint and tests (`yakogl-tools`, `yakogl-precompile`, the drivers crate
   with each shipped feature arm, the samples' check crates);
7. builds one Zephyr image per GPU with `-DYAKOGL_WERROR=ON` (needs
   `ZEPHYR_WS` and, for the AM62P, `driver/src/fw/pvr_fw_blob.h`), proves the
   precompiled tables deterministic and pinned, and reproduces the Mesa
   reference table byte for byte with `pco_shc` (needs `PCO_SHC`, `GLSLANG`,
   `GLSL2USC_CACHE`).

A row that could not run is a **gap**, printed in yellow and counted; a gap
makes the gate red, because a skipped step that looks green is how a blind
spot is born. The gate is green only at "N ok, 0 failed, 0 gap(s)". So a
review workspace without the Zephyr workspace, hal_nxp, pco_shc and the
firmware header cannot produce a green gate; it produces a red one with named
gaps, and the reviewer has to read which rows were red for a reason and which
were gaps. `tools/check.sh --quick` (the PowerVR row and the AM62P archive
only, about 5 seconds warm) is an edit-loop tool and never the landing gate.
`YAKOGL_CHECK_CONFIGS` narrows the configuration list for a change of known
reach; the image builds and tool tests still run.

The gate is run from a clean detached checkout of the exact commit, with the
firmware header copied in, so that nothing in a working tree can make it pass.

**What each stage proves.** Every number the project quotes comes from a real
GPU; software rendering is out of scope. After a commit is on `origin/main`,
the boards run it:

- The **common set** (`tools/cts/common-tree.txt`), one command per board
  (`tools/cts/cts-common.sh <board> build|run|tally`): glmark2, Slint with the
  window copied, Slint with the window a view into scan-out, and the board's
  gl_test with its default case list, each a Zephyr image booted from the
  board's own bootloader (U-Boot from the eMMC on the two Verdins with
  `BOARD_BOOT=uboot-mmc`, the firmware's tryboot on the Pi 4). This is what
  fills the gl_test / Slint / cluster table in `docs/HANDOFF.md` and the
  glmark2 table in `README.md`.
- The **dEQP-GLES2 trees**, the 17,165-case mustpass list: 17 images on the
  AM62P, one image with a 29-entry table and a selector on the i.MX8MP, 29
  images on the Pi 4, run by `tools/board.sh <board> tree`, tallied per case
  from the records, and diffed against the earlier tree. These fill the
  conformance table in `README.md` and the rows of `tools/cts/RESULTS.md`.

`tools/board.sh` is the one runner: it checks the image (header, footprint,
watchdog arm line), takes the board lock (one GL client per board; a busy
board is a refusal, exit 3), stages the image, boots it once, captures the
console or reads the SNVS record, decodes the records with `yakogl-tools
record` and prints one verdict line. A development boot (`kexec-dev`) is
marked DEV and is never a row. A failed board row is not automatically a
regression: the known defects (`docs/HANDOFF.md` "Known defects", each with
a GitHub issue) make some rows fail by design, so board stages report and a
person judges them against HANDOFF.md.

**How results are recorded.** Run directories go to
`records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/` with `INDEX.txt`, `meta.txt`
and the console; the directory is gitignored and every address in it is
redacted, and `BOARD_HOST` is never written. The numbers are then copied, by
hand, into the repository in the same commit or push as the result they
describe: `docs/HANDOFF.md` ("State of main", the open work in order, the
known defects), `tools/cts/RESULTS.md` (the measurement log; cells cite the
commit and the record directory), `tools/cts/STATUS.md` (the summary, in
Japanese) and `README.md`'s Status tables. Generated files the gate
reproduces are regenerated in the commit that moves them. Commit subjects are
`<area>: <what changed, as a result>` in lower case; the body explains the
design and names the records and commits it rests on.

**Secrets and machine facts.** Board addresses, UART paths, credentials and
the paths of external workspaces are environment only (`BOARD_HOST`,
`BOARD_UART`, `ZEPHYR_WS`, `HAL_NXP_DIR`, `PCO_SHC`, ...); the repository
defaults none of them to a path on one machine, and the PowerVR firmware
header is generated locally from TI's firmware and never committed.
