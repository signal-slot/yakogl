<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Device reachability: which node can use which board

Date: 2026-10-08. Plan #5 (remove the single point of failure on node nuc).
This file is read-only reporting: no board was leased, booted or logged into
to produce it, and nothing was changed on any device.

Method: two sources, kept apart.
- **Confirmed**: dated board sessions and build runs recorded in this
  repository (the commit author names the worker node, e.g. `ccfarm (nuc)`),
  plus one live network probe run on 2026-10-08 from this worker (DNS + TCP
  connect only, no login).
- **Unconfirmed**: everything else. The ccfarm tools that would have given
  lease statistics (`farm_status`, `list_resources`) were refused in this
  session, so lease counts and device presence could not be read.

Redaction rule (same as docs/HANDOFF.md): device names only. No addresses,
logins, ports or credentials appear here.

## Devices

| Device | What it is | How a board session uses it |
|---|---|---|
| `pi4-elegance` | Raspberry Pi 4 Model B board | ssh to the board; the debug UART is reached through a TCP serial bridge on another host, and `tools/board.sh pi4` needs a local character device, so the 2026-10-07 session used a small pty-to-TCP forwarder kept outside the repository |
| `verdin-am62p` | Verdin AM62P board | ssh to the board; console over the `usb-serial-1` serial-over-TCP bridge; boot by `uboot-mmc` (no netboot server needed) |
| `verdin-imx8mp` | Verdin i.MX8M Plus board | ssh only; no UART is wired for it; records come back through the board's nvmem |
| `usb-serial-1` | serial-over-TCP bridge carrying the AM62P console | two bridge ports exist; only the one wired to the SoC's main UART carries the console — the other is wired to the coprocessor UART and reads nothing |
| `usb1` | older serial bridge, reported dead in RESULTS.md (2026-10-04..06) | not used by current sessions; its host still answered on 2026-10-08 |

## Nodes

`nuc` (the lead's machine), `system76`, `XPS139340`, `Lepton`, `Lepton-local`.
The repository records the worker node in each commit author line. Two author
names, `Lepton` and `Lepton-local`, appear; the merged i.MX8MP plan #1 session
doc names its worker "Lepton (ccfarm-local)", and this worker's machine has
hostname `Lepton` while hosting the `Lepton-local` worker, so the table below
uses one row for both. Whether they are the same physical machine is
unconfirmed.

## Reachability table

Each cell says confirmed or unconfirmed, with the date of the evidence.
Evidence keys (E1..E9) are explained under the table.

| node | pi4-elegance | verdin-am62p | verdin-imx8mp | usb-serial-1 | usb1 |
|---|---|---|---|---|---|
| nuc | confirmed 2026-10-07 (E1) | confirmed 2026-10-07 (E2) | confirmed 2026-10-07 (E3) | confirmed 2026-10-04..07 (E4) | confirmed 2026-10-04 (E4) |
| system76 | unconfirmed (E5) | confirmed 2026-10-08 (E6) | confirmed 2026-10-08 (E7) | confirmed 2026-10-08 (E6) | unconfirmed |
| XPS139340 | confirmed 2026-10-08 (E8) | unconfirmed | unconfirmed | unconfirmed | unconfirmed |
| Lepton / Lepton-local | unconfirmed (E9) | confirmed 2026-10-08 (E9) | confirmed 2026-10-08 (E9) | confirmed 2026-10-08 (E9) | host answers 2026-10-08; role unconfirmed |

Evidence:

- **E1** nuc ran the Pi 4 common set and the plan #1 confirmation on the
  board: commits `33376cb`, `8e2a4ae`, `691cbd7`, `797166b`, `702797d`
  (2026-10-07), including the full common-set run recorded in
  `docs/benchmarks-baseline/` history.
- **E2** nuc ran the AM62P common set on the board: `520831d`..`b79ff7d`
  (turns 1..20, 2026-10-07).
- **E3** nuc ran the i.MX8MP common set baseline: `4ac3e36` (2026-10-07).
- **E4** the lead's runs from the main checkout (on nuc) used the `usb1`
  bridge first, then `usb-serial-1`, for the AM62P console (RESULTS.md,
  2026-10-04..06).
- **E5** system76's Pi 4 attempts ran nothing: `dbd1e88` and `5e28818`
  (2026-10-07/08) record missing inputs and every ssh attempt held for
  approval. No Pi 4 use from system76 is recorded.
- **E6** system76 ran `tools/board.sh am62p tree` on the board with a lease:
  ssh key login worked, the board rebooted and came back, and the leased
  bridge was connected (`docs/board-sessions/am62p-gles3-percase.md`,
  `64f04b3`, 2026-10-08). The console read zero bytes because the leased
  bridge port was wired to the coprocessor UART, not the main one.
- **E7** system76 ran 40 i.MX8MP boots with results: `4aa7095` (2026-10-08).
- **E8** XPS139340 logged into `pi4-elegance` with the leased key (login
  succeeded, board idle): `50a0b52` (2026-10-08). Nothing else from XPS is
  recorded.
- **E9** Lepton-local ran both Verdin plan #1 board sessions on 2026-10-08:
  AM62P images 1..7 (10,092 pass / 9 fail, `6f4cfe5`, `bf5c575` — that
  result is on an unmerged branch because it named a login and the bridge,
  so it must re-land redacted) and i.MX8MP 60 boots (`a6ceaf5`; the merged
  `docs/board-sessions/imx8mp-plan1.md` names the worker "Lepton
  (ccfarm-local)"). The seven AM62P images and the board firmware input are
  in this machine's dependency cache. A live probe from this machine
  (2026-10-08, DNS + TCP connect, no login): `verdin-am62p` and
  `verdin-imx8mp` resolve and answer on their ssh port; `usb-serial-1` and
  `usb1` resolve and their hosts answer; `pi4-elegance` does not resolve,
  which suggests the board is offline (MagicDNS resolves online peers).

Reachability today (2026-10-08) from nuc, system76 and XPS139340 was not
probed: this session could not run anything on other nodes. Their cells rest
on the dated history above.

### Network reachability is not login

Board ssh login works only with the key ccfarm provides with a lease. On
2026-10-08 a system76 worker without a lease reached `verdin-am62p`'s ssh
service but was refused login (`Permission denied (publickey,password)`,
`7dd1eef`/`45793d7`). So "the node can reach the device" and "the node can
log into the device" are different facts; every confirmed board use above had
a lease or the lead's own key.

## Which nodes can build images for which board

| board | nuc | system76 | Lepton / Lepton-local | XPS139340 |
|---|---|---|---|---|
| verdin-am62p | yes (2026-10-07, images built and ran) | yes (2026-10-08, dEQP stage 1 gles3+gles2 and image 7 built) | yes (2026-10-08, stage 1 and plan #1 images 1..7 built) | no (2026-10-08: no inputs; network refused) |
| verdin-imx8mp | yes (2026-10-07) | yes (2026-10-08, stage 1 and mini image) | yes (2026-10-08, stage 1 and GT image) | no (same) |
| pi4-elegance | yes (2026-10-07) | yes (2026-10-08, stage 1 for rpi_4b) | yes (2026-10-08, stage 1 for rpi_4b) | no (same) |

The AM62P firmware blob is not in the repository (it must never be
committed). A node needs it from the board (ssh fetch) or from a copy:
copies were fetched on nuc (lead's runs), on system76 (2026-10-08) and one is
in Lepton-local's dependency cache.

## Refused requests

The plan background reports, for 2026-10-08: `verdin-am62p` 21 refused
requests, `verdin-imx8mp` 6. **Unconfirmed here**: `list_resources` and
`farm_status` were refused in this session, so lease statistics could not be
read, and the repo does not record reasons. Contention recorded in the repo
that plausibly contributes (not verified as the cause):

- a leased bridge port wired to the wrong UART (wasted sessions, E6);
- "bridge busy (rc=3)" and "serial bridge held by the console MCP client"
  (am62p-gles3-percase notes, 2026-10-08);
- the board lock is per-machine (`/run/user/<uid>/yakogl-locks`, keyed by
  board host): two nodes running the same board do not share the lock, and
  only the board-side idle check catches the collision.

## If node X stops, these board sessions stop

- **nuc stops**: ccfarm's records stop (the plan background says records live
  only on nuc), so planning, triage and progress reporting stop; the main
  checkout and the lead's board runs stop. nuc is the only node confirmed to
  have run a full Pi 4 board session (2026-10-07, with the UART forwarder).
  Verdin board sessions continue: system76 and Lepton-local are confirmed for
  both Verdin boards (2026-10-08).
- **system76 stops**: lose one of the three builders for every board, the
  i.MX8MP gc-case10 runner, and one AM62P bridge runner.
- **Lepton / Lepton-local stops**: lose the only node confirmed to have run
  both Verdin plan #1 sessions with full inputs (2026-10-08), and the local
  copies of dEQP stage 1 for all three boards, the plan #1 AM62P images, and
  the board firmware input.
- **XPS139340 stops**: lose the only confirmed non-nuc login to
  `pi4-elegance` (2026-10-08).
- **usb-serial-1 stops**: AM62P console capture stops; `tools/board.sh am62p`
  refuses when the bridge is busy or unreachable, so AM62P tree runs cannot
  be tallied. i.MX8MP is unaffected (no UART). The Pi 4 console also comes
  from a TCP bridge on another host — which bridge, unconfirmed.
- **usb1 stops**: nothing as of 2026-10-08 (its bridge is dead); if it is
  the host of the Pi 4's UART bridge, Pi 4 console sessions would stop —
  unconfirmed.

## Known hazards for the next sessions

- Pi 4: `tools/board.sh pi4` needs a local UART character device; a node
  without one needs the pty-to-TCP forwarder trick (outside the repository)
  or a locally plugged probe. The 2026-10-07 nuc session also left a staged
  image and boot file on the Pi 4 board (removal was not permitted); the
  next Pi 4 session should clear them.
- AM62P: check which bridge port is wired to the main UART before a session;
  the wrong port reads zero bytes and the run is uncountable.
- A stale console buffer from a bridge can fake a first run's results (seen
  2026-10-07 on the Pi 4: the first run's numbers were invalid).

## What could not be confirmed, and why

- Lease statistics, device presence and the refused-request counts: the
  ccfarm tools that expose them (`farm_status`, `list_resources`) were
  refused on every attempt in this session.
- Today's reachability from nuc, system76 and XPS139340: no probe could be
  run on those nodes from here.
- Whether `pi4-elegance` was offline on 2026-10-08 (its name did not resolve
  from this machine).
- Whether `Lepton` and `Lepton-local` are one machine.
- Which host carries the Pi 4's UART bridge, and `usb1`'s role.
