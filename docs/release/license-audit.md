<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Licence / SPDX audit (public release)

Tool: `tools/license_audit.py`. Re-run:

    python3 -I tools/license_audit.py [--depth N] [--list CLASS] [--json] [--strict]
    python3 -I tools/license_audit.py --self-test

`--strict` exits non-zero when there are findings. It is not wired anywhere:
`tools/cts/cts-selftest.sh` (run by `tools/check.sh`) runs checks 23-25, the
tree-diff self-test, `--self-test` of this tool and the report-mode audit
below. The repository's own allowlist is `tools/license_audit.allow`,
read by default.

Run at `79c73ea` (the integration tip) plus this item's changes: 2431 tracked
files.

## Gate and open decision (strict mode)

`tools/cts/cts-selftest.sh` check 25 runs `python3 -I tools/license_audit.py`
in report mode over the repository. It fails only on a crash (non-zero exit)
or on `unknown-id` above 0, and prints the `missing-spdx` count as
information. Check 24 runs `--self-test` (the fixtures under
`tools/testdata/license_audit/`).

**Open owner decision:** whether the `missing-spdx` files get SPDX headers or
allowlist rules (`tools/license_audit.allow`). Until then `--strict` stays off
in every gate; turning it on with the current counts would fail. The
`missing-spdx` files are mostly the Qt/femtovg corpora, the dEQP-generated
fixtures and patches (table below).

Counts measured at the tip `d2a07a33` (`python3 -I tools/license_audit.py`):
2470 files, 1343 with an SPDX line, 342 allowlisted; `missing-spdx` 597,
`unknown-id` 0, `unused-license` 4, `third-party-mismatch` 0,
`third-party-unlabelled` 9 (information, not a finding), generated 57
(mismatch 0); findings 601 (597 + 4). The 587 / 591 figures in the sections
below were measured at the earlier tip `79c73ea` and have grown by 10 files.

## Output (summary counts, at `79c73ea`)

```
licence audit: 2431 files, 1315 with an SPDX line, 341 allowlisted
identifier use: Apache-2.0=1581, BSD-2-Clause=1, BSD-3-Clause=3, GPL-2.0-only=1, MIT=120
missing-spdx: 587
  of missing-spdx, with licence text in the header: 1
third-party-unlabelled: 9
unknown-id: 0
unused-license: 4
third-party-mismatch: 0
generated: 57 (mismatch: 0)
findings: 591
```

Before this item (same tip, no edits): `1217 with an SPDX line, 29 allowlisted,
missing-spdx 994, unknown-id 0, unused-license 4, third-party-mismatch 3,
generated-mismatch 3, findings 1004`.

Fixture test (`tools/testdata/license_audit/basic`):

```
$ python3 -I tools/license_audit.py --self-test
ok   basic
ok   exit status (0 without --strict, non-zero with it)
self-test: passed
```

`cargo build --features gpu-v3d,shaderc-online` succeeds (no Rust source header
was touched; only `.md`, `.conf`, `.yaml/.yml`, `.toml`, `.rst`, `.gitignore`
files got two comment lines).

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
| `driver/` | Remaining: 6 sample patch/asset files, `golden-linux-2026-09-21.txt` (see owner decisions). |
| `tools/` | Remaining: shader corpora (`qt`, `qt3d`, `femtovg`, `offarm`), patches, traces, `wedge-shaders`, `qpu_corpus.txt`, `usc_blobs.txt`. |
| `tests/` | Remaining: 11 dEQP-generated `deqp_random_dead_cube/*.frag|.vert`. |
| `LICENSES/` vs identifiers | Every identifier used has a text (`unknown-id: 0`). |

### Crate licence fields (Cargo.toml)

| Manifest | `license` |
|---|---|
| `Cargo.toml` (workspace) | `Apache-2.0`; the `yakogl` package `Apache-2.0 AND MIT` |
| `capi/Cargo.toml` | `Apache-2.0 AND MIT` |
| `driver/rust/Cargo.toml` | `Apache-2.0 AND BSD-3-Clause AND MIT` (BSD-3-Clause: NXP register data, NOTICE 10) |
| `tools/precompile`, `tools/yakogl-tools` | inherit the workspace (`Apache-2.0`) |
| `driver/samples/zephyr/{slint,triangle,cluster}/check` | `Apache-2.0` |
| `driver/samples/zephyr/slint/host/bevy_gl_rec`, `slint/nostd/fvg_demo`, `slint/nostd/nostd-workspace.Cargo.toml` | no `license` field (sample manifests, not published; owner decision whether to add one) |

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

Nothing below was changed and no licence was guessed.

| # | Item | Files | Evidence | Options |
|---|---|---|---|---|
| 1 | `tools/shaders/qt/`, `qt3d/` | 501 | GLSL ES text extracted from Qt Quick / Qt Quick 3D `.qsb` packs (e.g. `qt/24bittextmask.frag`, `qt3d/0119aeb276db508c.frag`: `struct buf`, `cbMain` blocks); no header. Qt is LGPL-3.0 / GPL-3.0 / commercial; some Qt shader files are BSD-3-Clause. | Decide per file, add the texts, or drop from the public tree and fetch at build time. Largest risk. |
| 2 | `tools/shaders/femtovg/`, `offarm/` | 49 | femtovg shaders (`#define SELECT_SHADER`, `EDGE_AA`); NOTICE 8 says femtovg is `MIT OR Apache-2.0`. `offarm` are variants of the same text. | Add a header naming femtovg's licence and holder, after checking the upstream file. |
| 3 | `tests/fixtures/deqp_random_dead_cube/*` | 11 | Shaders from dEQP's random shader generator (`dEQP_Position`, one-letter names); VK-GL-CTS is Apache-2.0 (NOTICE 5). Files are inputs to tests; header edit changes the source hash. | Allowlist as Apache-2.0 (CTS) if accepted. |
| 4 | `tools/cts/testdata/wedge-shaders/*` | 4 | `hsl_to_rgb`, `rgb_to_hsl` fragment/vertex: not written as probes by us as far as the text shows; origin unclear (Qt Quick 3D?). | Settle origin first. |
| 5 | Patches: `driver/samples/zephyr/slint/nostd/{slint,femtovg,glow}-*-nostd.patch`, `bevy-0.19.1-gles3-native.patch`, `glmark2/patches/libmatrix-util-zephyr.patch`, `tools/cts/cts-yakogl-{target,zephyr}.patch`, `tools/pco_shc/{meson-vulkan,usclib-tex.cl}.patch`, `tools/v3d/mesa-v3d_shc.patch`, `tools/vivante/mesa-{cs-golden,etna_shc}.patch`, `tools/pvr/{km/fwinit-step7,rgxstart/rgxstart}.diff` | 14 | A patch's licence is its patched project's (NOTICE 1e/5/8). REUSE: `.license` sidecars. Slint patches would use `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`. | Approve the sidecar text per patch. This also makes the two Slint LicenseRefs and `GPL-3.0-only` used. |
| 6 | `driver/samples/zephyr/slint/ui/assets/slint-logo.png` | 1 | Slint's logo; not ours. | Asset terms from Slint, or remove. |
| 7 | `tools/pco_shc/usc_blobs.txt` | 1 | pco captures (machine code compiled by Mesa's pco from Mesa's usclib, NOTICE 1a: MIT). | MIT or Apache-2.0 + MIT; decide. |
| 8 | `tools/v3d/qpu_corpus.txt` | 1 | The file does not state where its QPU instruction words come from (Mesa's v3d tooling is used elsewhere in `tools/v3d`, MIT). | Establish origin. |
| 9 | `driver/src/display/golden-linux-2026-09-21.txt` | 1 | Register dump captured from Linux's display driver on the board (GPL-only source of the driver; the values are the hardware's). NOTICE closing paragraph. | Facts or derivative: decide; keep out of the public tree if in doubt. |
| 10 | `tools/pvr/rgxstart/*.trace` | 2 | Traces of the DDK's own start/stop code (MIT/GPLv2). | Allowlist as ours or as `LicenseRef-Imagination-DDK-MIT`. |
| 11 | `LicenseRef-Imagination-DDK-MIT` unused | - | Files in `third_party/ti-img-rogue-ddk-25.2/` carry `@License Dual MIT/GPLv2` text and no SPDX line; NOTICE 4 cites the text file. | Sidecars or one `UPSTREAM` convention; the tool must be told. |
| 12 | `GPL-3.0-only`, `LicenseRef-Slint-*` unused | 3 | No header uses them any more (glmark2 shaders left the tree); only NOTICE 8 and the Slint patches (#5) refer to them. | Keep with #5, or drop the texts and NOTICE lines. |
| 13 | `GPL-2.0-only` used once | - | Only through the DDK `UPSTREAM` ("Dual MIT/GPLv2"). | Informational. |
| 14 | `.ccfarm/project.toml`, `website/og-image.png` | 2 | ccfarm's project file; an image. | Remove from the public tree, or sidecar for the image (author?). |
| 15 | `driver/docs/part1-ddk-fw-bringup.md` | 1 | Header added after its front matter (article metadata: title, tags). | Owner may want the article's licence stated. |
| 16 | The `*.json` default row | - | `driver/samples/zephyr/v3d_triangle/shaders/*.json` assumed ours. | Confirm upstream. |
| 17 | `third-party-unlabelled` (9) | 9 | `docs/license.rst`, `box.h`, `u_atomic.h`, XML/YAML inputs of Mesa; DDK `config_kernel.h`, `README`; the Zephyr PR's `LICENSE`. Covered by the licence files UPSTREAM lists. | Information only. |

`--strict` stays non-zero until items 1-12 are decided: 587 of the 591
findings are the Qt/femtovg corpora (550), the dEQP-generated fixtures (11),
patches (14), and the rest of the list above; `unused-license` is 4.

## Generated files (57)

All outputs were compared with the licence of their generator or upstream;
`generated-mismatch: 0`. Outputs that take the licence of their input
(`regs.rs` BSD-3-Clause from NXP's headers; `src/pvr/csbgen.rs`,
`csbgen_wrap.c` MIT from Mesa's csbgen) carry `generated-ok` rows. Files that
mix our code with Mesa-derived code (`pvr_pds_render_gen.h`,
`src/shaderc/online/powervr/shapes.rs`) carry `Apache-2.0 AND MIT` and a
"Ported from" line.

## Follow-up

1. Owner: items 1, 5 and 7-12 of the table (each blocks `--strict`).
2. After the owner decision: switch check 25 of `tools/cts/cts-selftest.sh`
   to `--strict`, or to a baseline of the decided count.
3. Not covered by the tool: copyright holders inside file headers (NOTICE is
   hand-kept); `tools/check.sh` is unchanged.
