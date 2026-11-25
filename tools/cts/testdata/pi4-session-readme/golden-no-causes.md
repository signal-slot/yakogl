<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 rows 2-5 (S1 S1a S1b S1c) at 01234567

commit: 0123456789abcdef0123456789abcdef01234567

Session `2026-11-pi4-gles3-01234567-aborts2`: rows 2-5 of `tools/cts/pi4-gles3-tree.txt` (4 images: S1 S1a S1b S1c).
Per-image files, relative to this directory: `<LABEL>.console.txt`, `<LABEL>.tally.txt`. Counts are the mustpass view of the tally's TOTAL line; '-' is unknown.

| row | image | Pass | Fail | NotSupported | other | run | GPU recoveries | hangs | verdict |
|---|---|---|---|---|---|---|---|---|---|
| 2 | S1 | 3 | 0 | 0 | 1 | - | - | - | STOPPED after 4 cases (dEQP ended the session early) |
| 3 | S1a | 0 | 0 | 0 | 1 | 0.8 s | 0 | 0 | STOPPED after 1 cases (dEQP ended the session early) |
| 4 | S1b | 5 | 0 | 0 | 0 | 3.0 s | 0 | 0 | DONE, all passed |
| 5 | S1c | 4 | 0 | 2 | 0 | - | - | - | complete |
