<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Does each board's dEQP-GLES2 tree run the whole mustpass list?

Host only, 2026-10-14, at integration `cd738b1`. No board was used and no
existing file was changed.

Roadmap stage 1 ends when all three boards show zero failures over the whole
dEQP-GLES2 tree. That proves conformance only if the tree lists run every
case of the mustpass list. This note checks that.

## Result

**All three tree lists cover all 17,165 mustpass cases exactly once. No case
is missing and no case is in more than one entry.** No proposed change to a
tree list is needed, so there is no hand-over note.

| Board | Tree list | Entries | Mustpass cases | Covered | Missing | In more than one entry | Patterns that match no mustpass case |
|---|---|---:|---:|---:|---:|---:|---:|
| am62p | `tools/cts/am62p-full-tree.txt` | 17 (images 1-17) | 17,165 | 17,165 | none | none | 0 |
| imx8mp | `tools/cts/imx8mp-full-tree.txt` | 29 (table entries 0-28) | 17,165 | 17,165 | none | none | 0 |
| pi4 | `tools/cts/pi4-full-tree.txt` | 29 (images AL ... XT) | 17,165 | 17,165 | none | none | 0 |

Per-entry mustpass counts (the sum of each row is 17,165):

- am62p, images 1-17: 1379, 994, 1172, 1084, 2523, 1318, 1631, 324, 409, 74,
  1060, 882, 1267, 709, 1173, 743, 423.
- imx8mp, entries 0-28: 9, 19, 1379, 994, 1172, 1084, 625, 1898, 1318, 1631,
  40, 136, 112, 36, 112, 74, 192, 90, 15, 1060, 863, 709, 246, 396, 1267, 602,
  571, 259, 256.
- pi4, images in file order (AL, D, E, F1, FB1, FB2, FF, H, LA, MISC, NS, RT,
  S1, S2, S3, S4, S5f, S5v, SMALL, T1, T2, T3, T4, T5, TF, U1, VA, DR, XT):
  57, 292, 352, 1923, 390, 319, 5, 654, 19, 70, 642, 200, 2949, 1910, 2171,
  2012, 98, 100, 227, 181, 248, 304, 67, 7, 17, 1267, 571, 101, 12.

## The mustpass source

The list the project uses is `tools/cts/gles2-mustpass.txt`: 17,165 lines,
17,165 distinct names, all `dEQP-GLES2.*`. It came in with the first commit
(`afb1633`), and no script regenerates it (`gen-gles3-mustpass.sh` is the
GLES3 one). `tools/cts/README.md` names it "the 17165 cases a GLES2
conformance run must pass"; `tools/cts/check-deqp-images.py` and
`host-deqp-run.sh` read it, and `docs/es2-cts-failures.md` counts against it.

The CTS checkout the scripts use is VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`
(`$CCFARM_DEPS_DIR/VK_GL_CTS` and `DEQP_WORK/cts` are both at it; it is the
commit `gen-gles3-mustpass.sh` names for the GLES3 list). The in-repo list was
compared with that commit's Android lists:

- The union of `android/cts/main/gles2-main-*.txt` (the 2020 list plus the 2021,
  2023 and 2025 increments; 2022, 2024 and 2026 are empty) has 14,371 cases.
  Every one is in the repo list.
- The repo list has 2,794 more. All 2,794 are names in
  `android/cts/main/src/gles2-failures.txt` (2,948 names), the file the Android
  lists leave out because those cases fail on some device. So the project's
  list is the Android list plus those cases: it is stricter than the Android
  one, not looser.
- 209 names in `gles2-failures.txt` and `gles2-test-issues.txt` are not in the
  repo list (keywords 93, `functions.invalid` 15, `texture.wrap` ... `etc1` and
  `pot` cases, preprocessor, linkage and others).

What this check does not show: that the repo list equals every case dEQP-GLES2
registers. That needs a case dump of the dEQP-GLES2 binary
(`--deqp-runmode=txt-caselist`), and no host binary was available. The 206
non-mustpass cases below suggest the registered tree is slightly larger than
the list.

## Method

The scripts expand a tree line the same way. A line is a dEQP `--deqp-case`
value: comma-separated patterns, `*` matches any text.
`check-deqp-images.py` `selects()` is `fnmatch.fnmatchcase` on each pattern of
`filt.split(',')`. `am62p-deqp-images.sh` builds one image per non-comment
line; `pi4-deqp-images.sh` reads the same format (`# image <LABEL>` lines are
comments); for the i.MX8MP, `imx8mp-deqp-build.sh` joins the non-comment lines
with `;` into `CONFIG_DEQP_CASE_TABLE`, and the deqp sample's `case_argument()`
(`driver/samples/zephyr/deqp/src/main.cpp`) runs entry *i* as
`--deqp-case=<entry i>`. So an entry is the unit, and the line split is the
same for all three boards.

The check (a throw-away Python 3 script, not committed; run with `python3 -I`):

1. read the mustpass names (non-empty lines);
2. read each tree list, dropping blank and `#` lines (one entry per line);
3. for each entry and each mustpass name, select it when any pattern of the
   entry matches (`fnmatch.fnmatchcase`, the same test as `selects()`);
4. count how many entries select each name: 0 is *missing*, more than 1 is
   *duplicated*;
5. also list the patterns that match no mustpass name (a typo would show here).

`tools/cts/check-deqp-images.py` already checks "selects every case" for
am62p; this check also counts duplicates and covers imx8mp and pi4.

Limits of the method: it uses mustpass names, not dEQP's own case list, so it
cannot see a case that dEQP registers and the list lacks (above). It does not
check that an image builds or that a case's result is captured.

## Comparison with recorded totals

| Board | Recorded total | Source | Mine |
|---|---|---|---|
| am62p | images 1-7: 10,101 mustpass cases; images 8-17: 7,064; sum 17,165 | `docs/es2-cts-failures.md` (AM62P rows) | 1379+994+1172+1084+2523+1318+1631 = 10,101; the other ten = 7,064; sum 17,165. Equal |
| imx8mp | 17,371 cases begun, 206 not in the mustpass list; 29 entries | `docs/es2-cts-failures.md`, `tools/cts/imx8mp-full-tree-29460f96.results.tsv` | Sum of "begun" over the 29 rows is 17,371; my mustpass count is 17,165; the difference is 206. Equal, and the per-entry difference is the extra cases (below) |
| pi4 | per-image counts in the `# image <LABEL>: <n>` comments, 17,165 in total | `tools/cts/pi4-full-tree.txt` | All 29 per-image counts equal mine; sum 17,165. Equal |

The i.MX8MP's 206 cases that are run but not in the mustpass list, by
table entry (begun minus mustpass): entry 2: 133, 5: 16, 7: 2, 15: 6, 18: 43,
21: 2, 28: 4; all other entries 0. The 4 of entry 28 are the ones
`docs/es2-cts-failures.md` says pass. They come from patterns that select a
whole group (`shaders.keywords.*`, `texture.wrap.*` ...), so they are cases
that dEQP registers in the group but the list does not hold. The list of
Android known-issue names outside the mustpass (209 names) has the same
shape (keywords, `functions.invalid`, `texture.wrap`, preprocessor), but the
two numbers differ by 3 and the names were not matched one by one.

The am62p and pi4 trees run those cases too (the same patterns), but no
record of theirs gives a begun total to compare. They do not count against the
mustpass pass rate.

## Excusals

- `tools/cts/es2-accepted.txt` does not exist at this commit
  (`tools/cts/tree-diff.py`, from the item `cts-tree-diff`, was not yet
  merged either). No case is excused by a file.
- The only known-unsupported set is in prose (`docs/es2-cts-failures.md`,
  `tools/cts/STATUS.md`): 34 NotSupported cases (multisample 16, negative_api
  texture 9, clip_control 8, multisampled_render_to_texture.readpixels 1),
  and on the AM62P the 16 multisample cases run on the extra `ms4` image
  (`tools/cts/am62p-ms4-tree.txt`, one entry `multisample.*`). The `ms4` table
  is not part of the 17-image tree; its 16 cases are also in image 17 of
  `am62p-full-tree.txt`, where they are NotSupported on the single-sampled
  config. So for the AM62P a full result takes image 17 plus `ms4`, and the
  `ms4` pass replaces image 17's 16 NotSupported. That is by design (stated in
  the `ms4` file's header), but the pass criterion for "zero failures" must
  say which of the two results counts.

## Proposed changes to the tree lists

None needed: coverage is complete and has no overlap. Two optional hardening
steps, as text only:

1. Add the exactly-once check for all three boards to `check-deqp-images.py`
   (it checks coverage for the am62p table; the duplicate count and the
   imx8mp and pi4 lists are not enforced by `tools/check.sh` today, only by the
   hand-written header comments). A future edit of a tree line then cannot
   drop or double a group unnoticed. This belongs to the item that owns
   `check-deqp-images.py` and `tools/cts/README.md` (`pub-license-gate`).
2. Get dEQP's own case list (`--deqp-runmode=txt-caselist` of a host
   `deqp-gles2` at VK-GL-CTS `93bca01`) and compare it with the mustpass list
   once, so the claim "mustpass equals what a conformance run must pass" is
   backed by the source and not only by the repo file.

## What was checked and what was not

Checked on the host: the expansion of the three lists against the 17,165
names, the per-entry counts against the recorded totals, and the mustpass list
against the Android lists of VK-GL-CTS `93bca01`. Not checked: any board
result, whether an image builds, the length limit of an i.MX8MP entry (the
tree's header says the longest is 871 of 2,035 characters; not re-measured),
and the full dEQP-GLES2 case list.
