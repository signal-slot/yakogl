<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Roadmap

YakoGL's work goes in five stages, in this order. A stage is done when its
criterion holds; until then it comes first, and later stages do not take board
time from it. Host-only work for a later stage may run ahead.

The three boards are the TI AM62P (Imagination PowerVR BXS-4-64), the NXP
i.MX 8M Plus (Vivante GC7000UL) and the Raspberry Pi 4 (Broadcom V3D 4.2).

The current state below quotes the README's Status section,
[docs/es2-cts-failures.md](docs/es2-cts-failures.md) (the ES 2.0 conformance
inventory) and [docs/benchmarks.md](docs/benchmarks.md) (the benchmark
inventory). Those files are updated with each board result; this one changes
when a stage's criterion or state changes.

## 1. OpenGL ES 2.0 conformance

**Criterion.** On each of the three boards, the dEQP-GLES2 mustpass list
(`tools/cts/gles2-mustpass.txt`, 17,165 cases) runs to completion against the
current tree with 0 Fail. A NotSupported or QualityWarning result counts only
where docs/es2-cts-failures.md section C lists it as acceptable.

**State.** The last full trees had 0 failures on the AM62P (17,141 pass,
2026-10-02) and on the i.MX8MP (2026-10-01), and 25 cube-map failures on the
Raspberry Pi 4 (2026-10-01). Fixes since then, including the one for the 25
cube-map cases, pass host tests but no board has run a full tree with them yet.

## 2. Benchmarks running correctly

**Criterion.** On each of the three boards: glmark2-es2 (800x600, the default
set of 33 scenes) runs every scene with 0 failures and every scene draws the
right picture; the Slint sample (`driver/samples/zephyr/slint/`) and the AGL
cluster demo (`driver/samples/zephyr/cluster/`) pass their colour-probe checks;
and the scores are recorded.

**State.** glmark2 runs all 33 scenes on every board (scores 457 on the AM62P,
183 on the Raspberry Pi 4, 37 on the i.MX8MP), but two scenes draw wrong:
terrain samples a black texture on every board, and refract misses its depth
pass on the AM62P and the Raspberry Pi 4; the fixes for both are tested on the
host only. Slint and the cluster demo pass their probe checks on all three
boards.

## 3. Public release

**Criterion.** The repository is published with a current CHANGELOG.md, this
README, CONTRIBUTING.md, SECURITY.md and this roadmap; a licence and SPDX audit
of every file with no open finding; a review of the tree and its history that
finds no private board details or credentials; and a website (`website/`) that
states the same status as the README.

**State.** CHANGELOG.md, CONTRIBUTING.md, SECURITY.md and the licence texts
(`LICENSE`, `LICENSES/`) are in the tree. The licence audit, the history
review and the website update are still to do.

## 4. OpenGL ES 3.0

**Criterion.** On each of the three boards, the dEQP-GLES3 mustpass list runs
with 0 Fail, under the same rule for NotSupported and QualityWarning as stage 1,
in a build that reports ES 3.0 (`YAKOGL_REPORT_ES2=OFF`).

**State.** A build can already report ES 3.0 (see the README's build options),
and the work on the ES 3.0 failures is being done on the host first. It moves
to the boards once stages 1 to 3 no longer need them.

## 5. More boards

**Criterion.** Each new board meets the stage 1 and stage 2 criteria the way
the three current boards do.

**State.** Not started. No board beyond the three above is supported yet.
