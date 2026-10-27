<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Licence / SPDX audit (public release)

Tool: `tools/license_audit.py`. Re-run:

    python3 -I tools/license_audit.py [--depth N] [--list CLASS] [--json] [--strict] [--fail-on CLASSES]
    python3 -I tools/license_audit.py --self-test

`--strict` exits non-zero on any finding. `--fail-on missing-spdx,unknown-id`
does so for the named classes only. `tools/cts/cts-selftest.sh` (run by
`tools/check.sh`) runs checks 23-25: the tree-diff self-test, `--self-test`
of this tool, and the audit over the repository (below). The repository's own
allowlist is `tools/license_audit.allow`, read by default.

Measured at `0dce654a8` (origin/main, 2026-10-26) plus the origin pass of
2026-10-26 ("Remaining findings" below): 6513 tracked files. The figures of
the SPDX header pass (`c2538996e81`, 6291 files, 79 `undecided`) are kept in
the sections that describe that pass.

## Gate (check 25)

`tools/cts/cts-selftest.sh` check 25 runs
`python3 -I tools/license_audit.py --fail-on missing-spdx,unknown-id,third-party-mismatch,generated-mismatch,unused-license`
over the repository. It fails when a tracked file has no SPDX line, no
allowlist row and no `.license` sidecar, when an identifier has no text in
`LICENSES/`, or when a third-party or generated file's licence disagrees with
its upstream or generator, or when a text in `LICENSES/` is named by no file.
A new source file takes the two SPDX lines (comment
syntax of its kind, as the neighbours do); a new data file takes a row in
`tools/license_audit.allow`. `undecided` is printed, not a failure. Check 24 runs `--self-test`: the fixtures under
`tools/testdata/license_audit/` (`basic` has a headerless `nohdr.rs`; `clean`
has headed files, an allowlisted file and an `undecided` patch) and the
`--fail-on` exit status, including a new headerless `.rs` added to the clean
tree, which must fail.

Counts (`python3 -I tools/license_audit.py`):

```
licence audit: 6513 files, 1684 with an SPDX line, 4622 allowlisted
LICENSES/: Apache-2.0, BSD-2-Clause, BSD-3-Clause, BSL-1.0, CC-BY-ND-4.0, GPL-2.0-only, GPL-3.0-only, LicenseRef-Slint-Royalty-free-2.0, LicenseRef-Slint-Software-3.0, MIT, Zlib
identifier use: Apache-2.0=6210, BSD-2-Clause=1, BSD-3-Clause=3, BSL-1.0=1, CC-BY-ND-4.0=1, GPL-2.0-only=1, GPL-3.0-only=1, LicenseRef-Slint-Royalty-free-2.0=1, LicenseRef-Slint-Software-3.0=1, MIT=208, Zlib=1
missing-spdx: 0
third-party-unlabelled: 13
undecided: 0
unknown-id: 0
unused-license: 0
third-party-mismatch: 0
generated: 59 (mismatch: 0)
findings: 0
```

`GPL-3.0-only`, `LicenseRef-Imagination-DDK-MIT`,
`LicenseRef-Slint-Royalty-free-2.0` and `LicenseRef-Slint-Software-3.0` left
`LICENSES/` and `NOTICE` (owner decision 5 of #1606; items 11 and 12 below):
no file named them. The origin pass brought `GPL-3.0-only` and the two Slint
texts back, unchanged, because the Slint patch's sidecar names them now
(item 5); `LicenseRef-Imagination-DDK-MIT` stays out. NOTICE 4 now points at the DDK's own `MIT-COPYING`,
vendored with it, and README and NOTICE 8 say that the Slint dependency
has its own licence. `third-party-unlabelled` (13: Mesa's `docs/license.rst`,
`box.h`, `u_atomic.h` and XML/YAML inputs; the DDK's `config_kernel.h` and
`README`; the Zephyr PR's `LICENSE`) is covered by the licence files the
`UPSTREAM` records list.

### How the 3075 `missing-spdx` files of the pass were settled

At the base the report said `missing-spdx: 3075`; the old figures of this
file (587 / 591 at `79c73ea`, 597 at `d2a07a33`) predate the Mali board-ref
sets, which added about 2400 files.

| Group | Files | Treatment |
|---|---|---|
| Own source and prose | 14 | SPDX header added (`Apache-2.0`, `Signal Slot, Inc.`): 10 READMEs and docs (`docs/es3/*.md`, `driver/samples/zephyr/*/replay/README.md`, `tools/mali/board-ref/*/README.md`), `tools/docs/check-refs.py`, `tri.c` and `tri_varying.c` (probes of `tools/mali/board-ref/2026-10-18/shaders/`), `.ccfarm/project.toml` |
| Board captures (`tools/mali/board-ref/`): `*.dmesg`, `*.log`, `*.stdout`, `*.stderr`, `*.txt`, `*.pandecode*`, `*.jpg` | 2189 | allowlist by kind, `Apache-2.0` (output of our runs). The directory left the tree on 2026-10-26 (#1606 step 5) and its 16 allowlist rows with it; the six excerpts the tests read moved to `tools/mali/board-dump/` (row `tools/mali/board-dump/*.txt`) |
| `tools/mali/testdata/` (probe shaders, disassembly, traces compared byte for byte), `panthor/uapi_pins.expected`, `isa-ref/board-2026-10-18.txt` | 217 | allowlist, `Apache-2.0` |
| `tools/cts/testdata/` (self-test inputs and expected output), `driver/rust/tests/gm-record/`, `tools/shaders/known-uncompilable-v3d.txt` | 64 | allowlist, `Apache-2.0` |
| `tests/fixtures/deqp_random_dead_cube/` | 11 | allowlist, `Apache-2.0` (VK-GL-CTS is Apache-2.0, NOTICE 5; the shader source is a test input) |
| Open provenance (`undecided`) | 580 | allowlist with `undecided`: no licence claimed (table below) |
| `src/mali/genxml/mesa_tests.rs`, `tools/mali/gen_isa_rs.py`, `tools/mali/gen_pack_rs.py` | 3 | had headers; their header text made them `generated-mismatch` against Mesa's MIT; `generated-ok` rows |

On the rebase onto main of 2026-10-24, 221 files had come in without an SPDX
line: 7 of our own sources took the header (`src/shaderc/online/mali/link.rs`,
three `docs/notes/mali-*.md`, `tools/mali/fvg_refusals.py`,
`test_fvg_refusals.py`, a board-ref `run1.sh`); 145 board captures
(`*.pam`, `*.rgba`, `*.kmsg`, `*.trace`, `*.xz`), 3 `tools/mali/board-dump/`
records and 66 host-harness fixtures (`tools/hostharness/glmark2/testdata/`)
took allowlist rows. The 501 `undecided` rows of the Qt shaders left with the
files (#1509): 580 became 79.

## How the tool decides

- A file's licence is the first `SPDX-License-Identifier:` in a comment in its
  first 60 lines (`//`, `/*`, `#`, `..`, `<!--`, ...). A tag inside a string
  does not count.
- A file that cannot carry a header has a REUSE `.license` sidecar or a row
  in an allowlist (`glob | SPDX | reason`; `DEFAULT_ALLOW` in the script and
  `tools/license_audit.allow`). `glob | generated-ok | reason` exempts a
  generated file whose licence is its input data's.
- Identifiers need `LICENSES/<id>.txt|.md`; `AND`/`OR`/`WITH` are split.
- `third_party/<dir>/` files are not edited, so most carry no SPDX line; the
  tool reads the licence text in the header and compares it with the
  `licence:` field of that directory's `UPSTREAM`.
- `tools/testdata/license_audit/` is not scanned (its fixtures are wrong on
  purpose).

## Scope check

| Area | Result |
|---|---|
| `src/`, `capi/`, `include/`, `zephyr/` | Every file has an SPDX line or an allowlist row; `include/KHR/khrplatform.h` via a row (MIT text in the file). |
| `driver/` | Headers, rows or sidecars; the sample patches have sidecars (item 5), `golden-linux-2026-09-21.txt` a row (item 9). |
| `tools/` | Own scripts and READMEs have headers; captures and byte-compared data are allowlisted; the `femtovg` and `offarm` shaders, `wedge-shaders`, traces, `qpu_corpus.txt` have rows naming their source; patches have sidecars; `usc_blobs.txt` has a header (items 2, 4, 5, 7, 8, 10). |
| `tests/` | `deqp_random_dead_cube/*` allowlisted (Apache-2.0, CTS). |
| `LICENSES/` vs identifiers | Every identifier used has a text (`unknown-id: 0`). |

### Crate licence fields (Cargo.toml)

| Manifest | `license` |
|---|---|
| `Cargo.toml` (workspace) | `Apache-2.0`; the `yakogl` package `Apache-2.0 AND MIT` |
| `capi/Cargo.toml` | `Apache-2.0 AND MIT` |
| `driver/rust/Cargo.toml` | `Apache-2.0 AND BSD-3-Clause AND MIT` (BSD-3-Clause: NXP register data, NOTICE 10) |
| `tools/precompile`, `tools/yakogl-tools` | inherit the workspace (`Apache-2.0`) |
| `driver/samples/zephyr/{slint,triangle,cluster}/check` | `Apache-2.0` |
| `tools/slint/bevy_gl_rec`, `examples/nostd/nostd-workspace.Cargo.toml` | no `license` field (sample manifests, not published; owner decision whether to add one) |

The `Apache-2.0` workspace field does not name the MIT parts of the `yakogl`
package; the package line `Apache-2.0 AND MIT` does. No field was changed.

## Fixed here (mechanical)

1. **Restored the tool and fixtures** from `ec9f59c`/`d9426d0` (the only
   version of the files on any ref; the four refs containing the commits hold
   no later change). Additions to the tool: the repository allowlist is read by
   default; `generated-ok` rows; `..` (reST) counts as a comment.
2. **SPDX headers added** (`Apache-2.0`, `Signal Slot, Inc.`) to 98 files
   plainly written for this project: 36 `.md` (docs, notes, READMEs), 43
   `.conf` (Zephyr `prj.conf`, `boards/*.conf`), 7 `.yaml` and 5 `.yml` (board
   and DTS binding files), 1 `.rst`, 3 `.toml`, 2 `.gitignore`, and one
   allowlist `.txt` (`tools/hostharness/allow-mesa-pds.txt`). No source file
   (`.rs/.c/.h/.py`) changed.
3. **`tools/license_audit.allow`** (new): rows for files that cannot take a
   header or must stay byte-identical, each with its reason:
   - project probe shaders under `tools/shaders/{es100,es300,gltest,
     gl_test_es100,internal}` and 21 top-level ones: the offline shader table
     hashes the source, so a header would break it. Apache-2.0.
   - our run outputs and lists (`tools/cts/*-tree.txt`, `*.qpa`,
     `cluster-run`, UART captures, `tests/fixtures/*.txt`, records hashes).
   - `*-mustpass.txt` (case names from the VK-GL-CTS mustpass lists): Apache-2.0.
   - Mesa csbgen outputs (7), `u_format_gen.h`, `v3d_packet_v42_pack.h`: MIT
     (NOTICE 1a/1b; `check.sh` regenerates them, so no edit).
   - `softfloat.c/.h` BSD-3-Clause and `xxhash.h` BSD-2-Clause inside
     `third_party/mesa-26.2.1/` (their headers say so).
   - three `generated-ok` rows (`regs.rs`, `src/pvr/csbgen.rs`,
     `csbgen_wrap.c`): their licence is their input data's.
4. **`LICENSES/BSD-2-Clause.txt`** added (used by `xxhash.h`).
5. **NOTICE corrected**: new entry 1e for the non-MIT files in the vendored
   Mesa (softfloat, xxhash, with holders and licences); the index lists
   BSD-2-Clause; two stale paths fixed (`src/pvr/blobs.rs` is
   `tests/reference/blobs.rs`; `tools/mkfwblob.sh` is
   `driver/tools/mkfwblob.sh`).

## Remaining findings: owner decisions

The SPDX header pass parked 79 files as `undecided` (no licence claimed). The
owner's rule of 2026-10-26: find the origin; if it cannot be established, the
file leaves the tree. Every item below was settled that way on 2026-10-26.
No licence was guessed: each row, sidecar or header names the source its
licence was read from. `undecided: 0`.

| # | Item | Files | What was found, and the source | What was done |
|---|---|---|---|---|
| 1 | `tools/shaders/qt/`, `qt3d/` | 501 | GLSL ES text from Qt Quick / Qt Quick 3D `.qsb` packs. | **Done**: removed from the tree (#1509, #1824). |
| 2 | `tools/shaders/femtovg/*.frag,*.vert`, `offarm/` | 49 (36 + 13) | femtovg 0.26.0's `src/renderer/opengl/main-fs.glsl` and `main-vs.glsl` as its `program.rs` assembles them, with the `NO_DISCARD` hunk of `femtovg-0.26.0-nostd.patch` (lines 590-608). Checked: the crate from crates.io (sha256 `2aaa216522a4fe16...`, `.cargo_vcs_info.json` sha1 `a9104b9079a3`), its two GLSL files plus that hunk, then `python3 -I tools/shaders/femtovg/verify_variants.py <dir>`: "36 variants expected; 0 problem(s)". `offarm/` is `fill_color.nodiscard.frag` plus our steps (`python3 -I tools/gen_offarm.py --check` passes). femtovg is `MIT OR Apache-2.0`, "Copyright 2021 femtovg contributors" (its `LICENSE-APACHE`); its README says it is ported from NanoVG. | Allowlist rows `MIT OR Apache-2.0` (the offline shader table hashes the text, so no header); NOTICE 8 names the files and the holder. |
| 3 | `tests/fixtures/deqp_random_dead_cube/*` | 11 | Shaders from dEQP's random shader generator. | Allowlisted as Apache-2.0 (CTS) with the pass. |
| 4 | `tools/cts/testdata/wedge-shaders/*` | 4 | Not Qt: the dEQP cases `shaders.algorithm.{hsl_to_rgb,rgb_to_hsl}_{vertex,fragment}`, built by `createExpressionCase()` and `LineStream(1)` in `modules/gles2/functional/es2fShaderAlgorithmTests.cpp` of VK-GL-CTS `93bca018` ("Copyright 2014 The Android Open Source Project", Apache-2.0). A script that applies that template to the file's `SHADER_OP_CASE` lines gives all four files byte for byte. | Allowlist row `Apache-2.0` (`tests/temps_survey.rs` reads them); NOTICE 5 names them. |
| 5 | The 14 patches (`examples/nostd/{slint,femtovg-0.26.0,glow-0.18.0}-nostd.patch`, `tools/slint/bevy_gl_rec/bevy-0.19.1-gles3-native.patch`, `driver/samples/zephyr/glmark2/patches/libmatrix-util-zephyr.patch`, `tools/cts/cts-yakogl-{target,zephyr}.patch`, `tools/mali/mesa-{glibc-once_flag,pan_shc}.patch`, `tools/pco_shc/{meson-vulkan,usclib-tex.cl}.patch`, `tools/v3d/mesa-v3d_shc.patch`, `tools/vivante/mesa-{cs-golden,etna_shc}.patch`) | 14 | Our patches. Each takes the licence of the files it patches (NOTICE 1f, 5, 8), read from those files: Mesa 26.2.1 (MIT; holders Imagination, Broadcom, Rob Clark, Collabora, Etnaviv Project, Zodiac, Intel), VK-GL-CTS (Apache-2.0, AOSP), glmark2 `22c527cb` `src/libmatrix/util.cc` (MIT, Linaro), the crates femtovg 0.26.0 and Bevy 0.19.1 (`MIT OR Apache-2.0`), glow 0.18.0 (`MIT OR Apache-2.0 OR Zlib`), Slint (`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`, SixtyFPS GmbH). Two corrections: `mesa-glibc-once_flag.patch` patches `src/c11/threads.h` and `impl/threads_posix.c`, which are **BSL-1.0** (yohhoy 2012, Yonggang Luo 2022; the same in Mesa 25.0.7, which the patch targets), not MIT; `libmatrix-util-zephyr.patch` said Apache-2.0 in its preamble, now MIT like its file. The Slint patch also adds our own `probe/` and `slint-ui-app/` files (Apache-2.0, one MIT). | A REUSE `.license` sidecar per patch with its expression and holders (ours plus upstream's). `LICENSES/` gains `BSL-1.0`, `Zlib` (SPDX license-list-data v3.27.0) and gets `GPL-3.0-only` and the two Slint texts back, as they were before `0dc8015b0` (see "Counts"). |
| 6 | `examples/slint/ui/assets/slint-logo.png` | 1 | Byte for byte Slint's `logo/slint-logo-full-dark.png` (sha256 `24c194ddf4c294ae...`, https://github.com/slint-ui/slint, master on 2026-10-26). Slint's `REUSE.toml` licenses `**/slint-logo-**.png` as `CC-BY-ND-4.0`, "Copyright © SixtyFPS GmbH"; its `logo/README.md` asks that the logo is not modified. Ours is unmodified. | Sidecar `CC-BY-ND-4.0`; `LICENSES/CC-BY-ND-4.0.txt` (SPDX v3.27.0); NOTICE 8. |
| 7 | `tools/pco_shc/usc_blobs.txt` | 1 | Written by `tools/pco_shc/gen_usc_blobs.py capture` from Mesa 26.2.1's pco compiling Mesa's `pvr_usc.c` and `usclib` (MIT, "Copyright © 2025 Imagination Technologies Ltd."). The headers `gen_usc_blobs.py render` makes from it already say `Apache-2.0 AND MIT`. | SPDX header `Apache-2.0 AND MIT` (the reader skips `#` lines); `capture` now writes the same three lines. `render --check` passes. The `capture --check` run needs a Mesa build with `capture_usc`, not on this host. |
| 8 | `tools/v3d/qpu_corpus.txt` | 1 | Its header names its origin: `tools/v3d/qpu_corpus.py` over a `glsl2v3d.py` run at `a87ed25c`. At that commit `tools/shaders` held 625 sources, 503 of them the Qt shaders of item 1, so most of its 37,427 words were Mesa's v3d_shc output for Qt text. | **Regenerated** without them: `glsl2v3d.py` failed on six gl_test sites it cannot read and now names them (`KNOWN_UNRESOLVED`); the new corpus (1090 of 1116 pairs, 8,893 words; three runs, same file) replaces the old one, and `roundtrip.rs` pins the new counts. Allowlist row `Apache-2.0` (our and femtovg's sources through Mesa's compiler). |
| 9 | `driver/src/display/golden-linux-2026-09-21.txt` | 1 | Register values read with `mmio_rd` on our board while Linux drove the panel (commit `0e7a8268f`, 2026-09-21; the bus-error line of our `golden_dump.sh` is in it). Hardware state, no driver text; `driver/docs/imx8mp-display-cold.md` says the GPL drivers were not read. | Allowlist row `Apache-2.0` (`driver/rust/src/model.rs` and `soc/imx8mp/media/tests.rs` read it with `include_str!`). |
| 10 | `tools/pvr/rgxstart/*.trace`, `rgxstart.diff`, `tools/pvr/km/fwinit-step7.diff` | 4 | Not DDK text: the traces are the register offsets and values our recording `sys_io` logged (`record_main.sh`, `rgxlayer_check.c`, `tools/pvr/rgxstart/README.md`); the diffs are `diff -u` of two of our records (`tools/check.sh` row "pvr rgxlayer_check" compares them byte for byte). | Allowlist rows `Apache-2.0`. |
| 11 | `LicenseRef-Imagination-DDK-MIT` unused | - | | **Done**: the text left `LICENSES/` (owner decision 5 of #1606). |
| 12 | `GPL-3.0-only`, `LicenseRef-Slint-*` unused | 3 | | Done with decision 5; item 5 brings them back, used by the Slint patch's sidecar. |
| 13 | `GPL-2.0-only` used once | - | Only through the DDK `UPSTREAM` ("Dual MIT/GPLv2"). | Informational. |
| 14 | `website/og-image.png`, `website/glmark2-am62p-phong.png`, `tools/cts/board-ref/glmark2-am62p-shading-phong/glmark2-phong.png` | 3 | `og-image.png` is ours: `python3 -I tools/website-og-image.py` (commit `91b24e17f`) writes the same bytes again, with SPDX tEXt chunks. The two glmark2 PNGs are the frame YakoGL drew of glmark2's `shading=phong` scene, a rendering of glmark2's `data/models/cat.3ds`; glmark2 is GPL-3.0 with no other licence for its data (its README). | `og-image.png`: allowlist row `Apache-2.0`. The two glmark2 PNGs are **removed**; the home page's GL band keeps its text without the figure, and the board-ref README keeps the run record and its picture value. |
| 15 | `driver/docs/part1-ddk-fw-bringup.md` | 1 | Our article (Qiita front matter; first commit `4402bcb3e`, Tasuku Suzuki). The header after the front matter says `Apache-2.0`, Signal Slot, Inc. | No change: the header states the licence. |
| 16 | The `*.json` default row | - | `tools/v3d/tri-shaders/*.json`: v3d_shc (Mesa) and our compiler's output for the v3d triangle sample's own probe shaders (commit `2e759bf7b`; the sample left the tree in `5a496fd50`, the shaders stay as test inputs). | No change: the default row (ours) holds. |
| 17 | `third-party-unlabelled` (13) | 13 | Mesa's `docs/license.rst`, `box.h`, `u_atomic.h`, XML/YAML inputs; DDK `config_kernel.h`, `README`; the Zephyr PR's `LICENSE`. Covered by the licence files UPSTREAM lists. | Information only. |

## Generated files (60)

All outputs were compared with the licence of their generator or upstream;
`generated-mismatch: 0` (three of the 60 through `generated-ok` rows, item 20). Outputs that take the licence of their input
(`regs.rs` BSD-3-Clause from NXP's headers; `src/pvr/csbgen.rs`,
`csbgen_wrap.c` MIT from Mesa's csbgen) carry `generated-ok` rows. Files that
mix our code with Mesa-derived code (`pvr_pds_render_gen.h`,
`src/shaderc/online/powervr/shapes.rs`) carry `Apache-2.0 AND MIT` and a
"Ported from" line.

## Follow-up

1. Done (2026-10-26): items 2, 4-10, 14-16 of the table; `undecided: 0`.
   Open for the owner: whether `undecided` becomes a `--fail-on` class of
   check 25, so that no new file can be parked again.
2. Done: `unused-license` is a `--fail-on` class of check 25 (items 11 and
   12 decided).
3. Not covered by the tool: copyright holders inside file headers (NOTICE is
   hand-kept).
