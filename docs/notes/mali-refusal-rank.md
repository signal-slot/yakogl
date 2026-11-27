<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Ranking Mali refusals by first refusal per case

`tools/mali/refusal-rank.py` (Python 3, stdlib only) counts, per refusal
text, the must-pass dEQP-GLES2 cases whose **first** refusal is that text.
Plan #387's acceptance criteria c2 and c5 use it; a refusal-line count (as in
the G310 summary) overstates texts that repeat inside one case.

## Usage

    python3 -I tools/mali/refusal-rank.py [--mustpass FILE] [--threshold N] RECORD_DIR...
    python3 -I tools/mali/refusal-rank.py --self-test

- `RECORD_DIR`: a record directory of a board run (`records/<board>/<name>/`,
  gitignored) or a single slice. Every `refusals.tsv` below it is read, in
  sorted path order.
- `--mustpass`: the list, default `tools/cts/gles2-mustpass.txt` (one case
  name per line; the list `tools/cts/mali-deqp-run.sh:204` plans from, read as
  `tools/cts/mali-deqp-tally.py:224` does, plus `#` comment lines skipped).
  Refused cases outside it are left out and reported as a count.
- `--threshold N` (default 100, the c2/c5 bar): exit 1 if any text blocks N
  or more cases, 0 otherwise; exit 2 for a usage or input error.
- Output: a markdown table (rank, text, stage `vs`/`fs`/`both`/`-`, cases
  blocked, share of the total) and a total line. A text over the threshold is
  also named on stderr.

## Input format (read from the writers)

- `tools/cts/mali-deqp-run.sh:500-502` (NanoPC-T6 slices) writes the header
  `case<TAB>refusal`, then one row per case from `ref.append` at `:491`:
  the case and `qpa_refusal(body)` (`:283-293`), the **first** shader or
  program info log of the case's log that says "refused" (or a link refusal),
  whitespace folded and cut at 600 characters (`field(..., 600)`). Format
  documented at `:92-101`.
- `tools/cts/imx95-uart-slice.py:462,469` (Verdin iMX95) writes the same
  header and rows (`ref = ["case\trefusal"]`), the text coming from the
  console's `glCompileShader`/`glLinkProgram` line (format at `:67-71`).

A case has at most one row per file, so "first refusal" is the writer's
choice. If a case appears in more than one file (a rerun slice), the later
file wins, as in `tools/mali/refusal-compare.sh`; list reruns after what they
rerun. Should a file hold two rows for one case, the first is kept.

## Text

`... refused this <stage> shader: Mali(<why>)` becomes `<why>` with the stage
in its own column (`both` when one text refuses in both stages); any other
text, such as a link refusal without that phrase, is kept whole. This is the
cause that `refusal-compare.sh` builds, minus the stage prefix, so a text cut
at a different length is a different row.

## Self-test

`--self-test` runs on `tools/mali/testdata/refusal_rank/` (made-up cases and
texts): first-per-case, a rerun overriding, must-pass filtering, ranking,
table form, `--threshold` (exactly-at-N fails, above passes, default 100
passes), and the empty-directory and bad-header errors.
