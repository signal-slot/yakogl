<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP whole dEQP-GLES2 tree, 2026-10-14, origin/main 835cf9cc

## State of main
The i.MX8MP whole dEQP-GLES2 tree on `835cf9cc` (29 entries, `records/imx8mp/deqp-835cf9cc`):
17,371 begun, 17,272 pass, **46 fail**, 34 not supported, 19 other, 0 no-result. All 46 are
`GL_BGRA_EXT` renderbuffer cases of entry 21 (`fbo`); every other entry equals the `32778b57`
run. README's Status row and `tools/cts/STATUS.md` say 46, not 0.

## Known defects
The 46 `GL_BGRA_EXT` renderbuffer failures stay open on `835cf9cc`: the report-es2
renderbuffer allowlist of `8bbf9ba2` names `GL_BGRA8_EXT` but not the unsized `GL_BGRA_EXT`
(`docs/board-sessions/imx8mp-es2-final.md`, `da842c6`). Fixing the allowlist should bring the
i.MX8MP back to 0 fail; the same allowlist is why the Pi 4 shows the same 46 failures at
`20a48791` (`1344748`).

## Next board session
Re-run entry 21 (`fbo`, with `TREE_ROWS` as in `tools/board.sh`) after the allowlist fix
and expect 711 of 711.
