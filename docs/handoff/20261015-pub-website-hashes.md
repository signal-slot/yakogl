<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pub-website-hashes, 2026-10-15, website/data.js cites commits of this history

## State of main
`website/data.js` no longer cites the five commits of the rewritten history
(`d60b9d76`, `fe5e8e73`, `1fa941e3` twice, `031de9a2`). `tools/website-check.sh`
is unchanged. `website/index.html` and `website/status.html` did not repeat them.

| entry | old hash | new hash | backing record |
|---|---|---|---|
| `deqp2.am62p.previous` | `d60b9d76` | dropped | 17,141 / 0 / 19 / 5 has no record here: `docs/board-sessions/am62p-es2-final-r2.md` (`63efed8e`) reads 17,141 / 0 / 18 NotSupported / 5 QW / 1 CompatW |
| `deqp2.pi4.previous` | `fe5e8e73` | dropped | no record in this history holds 17,075 / 25 fail; `docs/board-sessions/pi4-es2-final-r2.md` only cites the number |
| `glmark2.pi4` | `1fa941e3` | `d64e995b` | `docs/board-sessions/pi4-common-int.md`: score 183, 33 / 0 / 0, 0 GL errors, 96 MiB carve-out (2026-10-14) |
| `slint.pi4` | `1fa941e3` | `d64e995b` | the same record: SO and SV probes pass |
| `cluster.pi4` | `031de9a2` | `d64e995b` | the same record: 192.6 Mpx/s, 139.3 fps, 34 / 34 (was 193.4 / 139.8) |

Other changes in `data.js`: the glmark2.pi4 note now says terrain 3 fps and
refract 2 fps (the record's finding 1); `deqp2.am62p.integration`'s note says
"the 2026-10-02 whole tree" instead of `d60b9d76`.

## Open work
- `site.js` lists the `.previous` rows by key; the two dropped keys simply
  no longer render.
- `tools/cts/RESULTS.md` and `docs/es2-cts-failures.md` still cite the old
  hashes in prose; the checker does not scan them.

## Known defects
- The cluster Pi 4 figure moved (193.4 -> 192.6 Mpx/s) because the old one has
  no record here; it is one 60 s run.

## Next board session
None needed.
