<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Ranking Mali refusals from a host dEQP measure

`tools/mali/host-rank.py` (Python 3, stdlib only) counts, per refusal text,
the must-pass dEQP-GLES2 cases whose **first** refusal on the host is that
text. It is the host twin of `tools/mali/refusal-rank.py`
(`docs/notes/mali-refusal-rank.md`) and serves plan #387's acceptance
criterion c2. HOST-ONLY: compile and link outcomes of the host library, no
render.

## Usage

    python3 -I tools/mali/host-rank.py [--mustpass FILE] [--threshold N] OUT_DIR
    python3 -I tools/mali/host-rank.py --self-test

- `OUT_DIR`: the output directory of `tools/mali/deqp-host-measure.sh`
  (`--out`, default `$TMPDIR/deqp-host-measure-<commit>`). `measure.tsv` is
  read; `summary.md` must be beside it (a wrong directory is an error).
- `--mustpass`: default `tools/cts/gles2-mustpass.txt`, as refusal-rank.py.
- `--threshold N` (default 100): exit 1 if any text blocks N or more cases
  (exactly N fails), 0 otherwise, 2 for a usage or input error. A text over
  the threshold is also named on stderr.

## Input

`measure.tsv`: header `case, board_status, board_family, host_status, stage,
host_family, shape, host_refusal, next_family`, one row per case. A row with
a non-empty `stage` (`vs`, `fs`, `link`, `be`) is a case with a host refusal;
`host_refusal` is its first refusal, already cut by the measure script to
the `Mali(Valhall: ...)` reason (cut at ` is not lowered`, a link refusal
without the case's own varying). It is used as the text unchanged. Rows
without a stage (Pass, NotRun, crash) are only counted. A case twice in the
file counts once, the first row.

## Output

A markdown table: rank, text, stage (`both` when one text refuses in more
than one stage), must-pass cases blocked, share; a total line; then the
number of must-pass cases without a host refusal and of cases outside the
must-pass list.

## Difference from refusal-rank.py

- Input: `measure.tsv` of one host run, not the `refusals.tsv` files of board
  slices; no rerun merging.
- Text: the measure script's short form, not refusal-compare's cause, so a
  text can differ from the board ranker's by the cut only. The stage column
  holds `link` and `be` as well as `vs`/`fs`.
- The first refusal is the measure script's (`first_refusal`: first program,
  vertex before fragment, then the link log), not the board writer's.

## Self-test

`--self-test` runs on `tools/mali/testdata/host_rank/` (made-up cases):
ranking, first row per case, stage `both`, must-pass filtering, the no-refusal
count, `--threshold` (exactly-at-N fails, above passes, default passes), and
the missing-`summary.md` and bad-header errors.
