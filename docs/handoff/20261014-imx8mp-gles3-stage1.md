<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# imx8mp-gles3-stage1, 2026-10-14, plan #19 item stage1-imx8mp

Base: main `72ee696`. Scripts: `tools/cts/cts-build-zephyr.sh`,
`tools/cts/imx8mp-deqp-stage1.sh`, `tools/cts/imx8mp-gles3-artefacts.sh`.
Docs: `docs/board-sessions/imx8mp-deqp-stage1-gles3.md`,
`docs/es3-board-recipe-imx8mp.md`, `docs/es3-zephyr-env.md`.

## State of main

The i.MX8MP dEQP-GLES3 table (`tools/cts/imx8mp-gles3-tree.txt`) covers every
`gles3-mustpass.txt` case once (`check-imx8mp-gles3-tree.py`). The GLES3
stage 1 and the table image were built at `ba8b242a`. ES 2.0 stays the
default: only `--gles3` turns `REPORT_ES2` off. No board run was made.

## Open work

- The board run: `board-imx8mp` follows `docs/es3-board-recipe-imx8mp.md`.
- The image is named `es3-<hash8>` after the commit it was built from. It
  must be rebuilt at the commit the board runs (`tools/cts/imx8mp-gles3-artefacts.sh es3-<hash8>`
  prints its size and sha256).

## Known defects

- `cts-build-zephyr.sh` now drops a stale CMake cache and prints the failing
  ninja step. The same fix applies to the AM62P and Pi 4 stage 1 builds.
- Failing GLES3 case names print as `crc:<hex>`, not as dEQP names. Map them
  with the table.
- `tools/cts/am62p-deqp-images.sh` lacks `-DCONFIG_YAKOGL_REPORT_ES2=n`.

## Next board session

i.MX8MP (`verdin-imx8mp`): the 133 table rows `e00`..`e132`, as in
`docs/es3-board-recipe-imx8mp.md` section 3.
