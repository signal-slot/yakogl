<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Device reachability: which node can use which board

Date: 2026-10-08, updated 2026-10-09. Plan #5 (remove the single point of
failure on node nuc). This file is read-only reporting: no board was leased,
booted or logged into to produce it, and nothing was changed on any device.

Three sources, kept apart:
- **Repo history**: dated board sessions and build runs recorded in this
  repository (the commit author names the worker node, e.g. `ccfarm (nuc)`).
- **ccfarm records** (`farm_status`, `list_resources`, `ask_farm`, read
  2026-10-08): node list, per-node ssh probes, lease statistics.
- **Live probe** from this worker on 2026-10-08 (DNS + TCP connect only,
  no login).

Redaction rule (same as docs/HANDOFF.md): device names only. No addresses,
logins, ports or credentials appear here.

## Devices

| Device | What it is | How a board session uses it |
|---|---|---|
| `pi4-elegance` | Raspberry Pi 4 Model B board (target `rpi4`) | ssh to the board; the debug UART is reached through a TCP serial bridge on another host, and `tools/board.sh pi4` needs a local character device, so the 2026-10-07 session used a small pty-to-TCP forwarder kept outside the repository. The board's tailnet name differs from the resource id: probing the id name does not resolve, probing the registered name resolves and answers ssh (2026-10-08) |
| `verdin-am62p` | Verdin AM62P board (target `am62p`) | ssh to the board; console over the `usb-serial-1` serial-over-TCP bridge; boot by `uboot-mmc` (no netboot server needed) |
| `verdin-imx8mp` | Verdin i.MX8M Plus board (target `imx8mp`) | ssh only; no UART is wired for it; records come back through the board's nvmem |
| `usb-serial-1` | serial-over-TCP bridge carrying the AM62P console (target `am62p`) | two bridge ports exist; only the one wired to the SoC's main UART carries the console — the other is wired to the coprocessor UART and reads nothing |
| `usb1` | older bridge host (target `rpi4`) | registered but unusable today: no capabilities, no keys installed, and ccfarm records it unreachable from every node (2026-10-08); a TCP probe from this machine the same day got connection refused, so the tailnet endpoint answers but no ssh service accepts |

## Nodes

`farm_status` (2026-10-08) lists five online worker nodes: `nuc` (the lead's
machine), `system76`, `XPS139340`, `Lepton` (Claude worker) and
`Lepton-local` (local-model worker). `Lepton` and `Lepton-local` have
identical login environments and toolchain lists, and the daemon serving this
`Lepton-local` session marks `Lepton` as its own node — consistent with one
machine running both workers — but no node's notes state this outright
(`ask_farm` could not confirm it), so the table uses one row for both.

## Reachability table

Each cell says confirmed or unconfirmed, with the date and kind of evidence:
*session* = a board session ran from that node; *probe* = ccfarm's own ssh
probe (network + key login verified from that node, recorded 2026-10-08) or
this worker's TCP probe. Evidence keys (E1..E12) are explained below.

| node | pi4-elegance | verdin-am62p | verdin-imx8mp | usb-serial-1 | usb1 |
|---|---|---|---|---|---|
| nuc | session 2026-10-07 (E1); probe 2026-10-08 (E10) | session 2026-10-07 (E2); probe 2026-10-08 (E10) | session 2026-10-07 (E3); probe 2026-10-08 (E10) | used 2026-10-04..07 (E4) | unreachable from every node, confirmed 2026-10-08 (E10) |
| system76 | probe 2026-10-08 (E10); no session ran (E5) | session 2026-10-08 (E6); probe 2026-10-08 (E10) | session 2026-10-08 (E7); probe 2026-10-08 (E10) | leased and used 2026-10-08 (E6) | unreachable, confirmed 2026-10-08 (E10) |
| XPS139340 | session 2026-10-08 (E8); probe 2026-10-08 (E10) | probe 2026-10-08 (E10) | probe 2026-10-08 (E10) | unconfirmed | unreachable, confirmed 2026-10-08 (E10) |
| Lepton / Lepton-local | probe 2026-10-08: registered name resolves, ssh port open (E11); ccfarm probe (E10) | session 2026-10-08 (E9); probe 2026-10-08 (E10) | session 2026-10-08 (E9); probe 2026-10-08 (E10) | host answers 2026-10-08 (E11); used 2026-10-08 (E9) | unreachable, confirmed 2026-10-08 (E10) |

Evidence:

- **E1** nuc ran the Pi 4 common set and the plan #1 confirmation on the
  board: commits `33376cb`, `8e2a4ae`, `691cbd7`, `797166b`, `702797d`
  (2026-10-07).
- **E2** nuc ran the AM62P common set on the board: `520831d`..`b79ff7d`
  (turns 1..20, 2026-10-07).
- **E3** nuc ran the i.MX8MP common set baseline: `4ac3e36` (2026-10-07).
- **E4** the lead's runs from the main checkout (on nuc) used the `usb1`
  bridge first, then `usb-serial-1`, for the AM62P console (RESULTS.md,
  2026-10-04..06).
- **E5** system76's Pi 4 attempts ran nothing: `dbd1e88` and `5e28818`
  (2026-10-07/08) record missing inputs and every ssh attempt held for
  approval.
- **E6** system76 ran `tools/board.sh am62p tree` on the board with a lease:
  ssh key login worked, the board rebooted and came back, and the leased
  bridge was connected (`docs/board-sessions/am62p-gles3-percase.md`,
  `64f04b3`, 2026-10-08). The console read zero bytes because the leased
  bridge port was wired to the coprocessor UART, not the main one.
- **E7** system76 ran 40 i.MX8MP boots with results: `4aa7095` (2026-10-08).
- **E8** XPS139340 logged into `pi4-elegance` with the leased key (login
  succeeded, board idle): `50a0b52` (2026-10-08).
- **E9** Lepton-local ran both Verdin plan #1 board sessions on 2026-10-08:
  AM62P images 1..7 (10,092 pass / 9 fail, `6f4cfe5`, `bf5c575` — that
  result is on an unmerged branch because it named a login and the bridge,
  so it must re-land redacted) and i.MX8MP 60 boots (`a6ceaf5`; the merged
  `docs/board-sessions/imx8mp-plan1.md` names the worker "Lepton
  (ccfarm-local)"). The seven AM62P images and the board firmware input are
  in this machine's dependency cache.
- **E10** ccfarm's resource records (read 2026-10-08): each of the five
  nodes has its ssh key installed on all three boards, and login was
  verified from every node to every board. `usb1` is recorded unreachable
  from every node with no keys installed.
- **E11** live probe from this machine (2026-10-08, DNS + TCP connect, no
  login): the Pi 4's registered tailnet name resolves and its ssh port is
  open (the resource id name does not resolve); `verdin-am62p` and
  `verdin-imx8mp` resolve and answer on ssh; `usb-serial-1` and `usb1`
  resolve and their hosts answer (ssh refused on `usb1`).
- **E12** `farm_status` (2026-10-08): five nodes online, all worker-enabled;
  node and toolchain details under "Build capability" below.

Reachability of the two bridge hosts from nuc, system76 and XPS139340 on
2026-10-08 was not probed directly (this session cannot run probes on other
nodes); their board cells rest on E10, and the bridge cells on E4/E6/E11.

### Network reachability is not login

Board ssh login needs the node's key on the board. On 2026-10-08 a system76
worker without a lease reached `verdin-am62p`'s ssh service but was refused
login (`Permission denied (publickey,password)`, `7dd1eef`/`45793d7`). As of
the ccfarm records read the same day (E10), every node's key is installed on
all three boards and login is verified from every node, so that gap is
closed; a lease is still what makes use exclusive and ordered.

## Lease statistics (ccfarm records, 2026-10-08)

| device | leases granted | median lease | utilization, last 24 h | refused requests, last 24 h |
|---|---|---|---|---|
| `pi4-elegance` | 272 | ~0 s | 0.3 % | 0 |
| `verdin-am62p` | 213 | ~0 s | 15 % | 39 |
| `verdin-imx8mp` | 21 | ~5.5 min | 29 % | 6 |
| `usb-serial-1` | 8 | ~6 min | 7 % | 0 |
| `usb1` | 3 | ~41 s | 0.3 % | 0 |

At the reading time, `verdin-am62p` and `usb-serial-1` were held under one
exclusive lease by a system76 task; `verdin-imx8mp` and `pi4-elegance` were
free. The very short medians on `pi4-elegance` and `verdin-am62p` show most
leases are brief checks, not sessions.

## Refused requests

Confirmed from ccfarm records (2026-10-08): `verdin-am62p` 39 refused
requests in the last 24 h, `verdin-imx8mp` 6, all other devices 0. The plan
background reported 21 and 6 earlier the same day; the AM62P count had grown
to 39 by the reading. Reasons are not recorded in ccfarm's records. What the
records do show, consistent with contention (not verified as the sole
cause): board leases are exclusive, so requests for a board held by another
task are refused while it is held — at the reading time the AM62P board and
its bridge were both held by one system76 task. Repo docs record related
contention: "bridge busy (rc=3)" and "serial bridge held by the console MCP
client" (am62p-gles3-percase notes, 2026-10-08); and the board lock is
per-machine (`/run/user/<uid>/yakogl-locks`, keyed by board host), so two
nodes running the same board do not share the lock — only the board-side
idle check catches the collision.

## Which nodes can build images for which board

Confirmed by dated build runs:

| board | nuc | system76 | Lepton / Lepton-local | XPS139340 |
|---|---|---|---|---|
| verdin-am62p | yes (2026-10-07, images built and ran) | yes (2026-10-08, dEQP stage 1 gles3+gles2 and image 7 built) | yes (2026-10-08, stage 1 and plan #1 images 1..7 built) | no (2026-10-08: no inputs; network refused) |
| verdin-imx8mp | yes (2026-10-07) | yes (2026-10-08, stage 1 and mini image) | yes (2026-10-08, stage 1 and GT image) | no (same) |
| pi4-elegance | yes (2026-10-07) | yes (2026-10-08, stage 1 for rpi_4b) | yes (2026-10-08, stage 1 for rpi_4b) | no (same) |

Registered environment readiness (`farm_status`, 2026-10-08) — what each
node has stored for yakogl builds, which decides what a future worker can
build without a setup turn:

- `system76`: the richest set (dEQP stage 1 and work dirs, CTS, glmark2,
  hal_nxp, i.MX8MP output, slint/nostd dirs, Noto Sans, Zephyr workspace,
  SDK and venvs, the aarch64 cross-compiler).
- `Lepton-local`: a rich set (dEQP stage 1 and work dirs, CTS, VK-GL-CTS,
  glmark2, hal_nxp, i.MX8MP output, Mesa source, Zephyr workspace, SDK and
  venvs, cluster dir).
- `XPS139340`: partial (Zephyr workspace, SDK and venv, hal_nxp, cluster
  dir, nightly toolchain) — enough for Zephyr samples, not for dEQP images;
  its 2026-10-08 session could not use even these (allowed-directory
  limits, `50a0b52`).
- `nuc` and `Lepton`: no stored values in ccfarm's records; nuc's builds ran
  with the lead's interactive shell values.
- Toolchains advertised: only `Lepton`/`Lepton-local` list `zephyr`;
  `XPS139340` lists docker and many rust toolchains; `nuc` lists neither
  docker nor zephyr.

The AM62P firmware blob is not in the repository (it must never be
committed). A node needs it from the board (ssh fetch) or from a copy:
copies were fetched on nuc (lead's runs), on system76 (2026-10-08) and one is
in Lepton-local's dependency cache.

## If node X stops, these board sessions stop

- **nuc stops**: ccfarm's records stop — the repository's record home is nuc
  and it was reported degraded on 2026-10-08, with its only follower
  (XPS139340) having applied none of the record entries (lag 169847), so
  planning, triage and progress reporting stop until the home moves; the
  main checkout and the lead's board runs stop. Verdin board sessions
  continue: system76 and Lepton-local are confirmed for both Verdin boards
  (2026-10-08), and all nodes now have board logins (E10).
- **system76 stops**: lose the richest build environment, the i.MX8MP
  gc-case10 runner, and one AM62P bridge runner.
- **Lepton / Lepton-local stops**: lose the only node confirmed to have run
  both Verdin plan #1 sessions with full inputs (2026-10-08), the only node
  advertising a zephyr toolchain, and the local copies of dEQP stage 1 for
  all three boards, the plan #1 AM62P images, and the board firmware input.
- **XPS139340 stops**: lose the largest free Claude capacity (11 slots, 75 %
  of its weekly window unused on 2026-10-08) and the only confirmed non-nuc
  Pi 4 login.
- **usb-serial-1 stops**: AM62P console capture stops; `tools/board.sh am62p`
  refuses when the bridge is busy or unreachable, so AM62P tree runs cannot
  be tallied. i.MX8MP is unaffected (no UART). The Pi 4 console also comes
  from a TCP bridge on another host — which bridge, unconfirmed (`ask_farm`
  2026-10-09: no node knows).
- **usb1 stops**: nothing today (unreachable from every node, no keys, no
  capabilities); if it is the host of the Pi 4's UART bridge, Pi 4 console
  sessions would stop — unconfirmed.

## Known hazards for the next sessions

- Pi 4: `tools/board.sh pi4` needs a local UART character device; a node
  without one needs the pty-to-TCP forwarder trick (outside the repository)
  or a locally plugged probe. The 2026-10-07 nuc session also left a staged
  image and boot file on the Pi 4 board (removal was not permitted); the
  next Pi 4 session should clear them.
- Pi 4 name: probe the board by its registered tailnet name, not by the
  resource id (the id name does not resolve).
- AM62P: check which bridge port is wired to the main UART before a session;
  the wrong port reads zero bytes and the run is uncountable.
- A stale console buffer from a bridge can fake a first run's results (seen
  2026-10-07 on the Pi 4: the first run's numbers were invalid).
- Board leases are exclusive: a held board refuses other requests (39
  refusals on the AM62P in 24 h), and the per-machine lock does not protect
  against another node's session.

## What could not be confirmed, and why

- Which host carries the Pi 4's UART bridge, and `usb1`'s role: `ask_farm`
  (2026-10-09) — no node's notes know it.
- Whether `Lepton` and `Lepton-local` are one physical machine: the node
  records suggest it (identical login environments, same daemon serves
  both) but no note states it.
- Reachability of the bridge hosts (`usb-serial-1`, `usb1`) from nuc,
  system76 and XPS139340 on 2026-10-08: no probe could be run on those
  nodes from here (ccfarm probes cover the boards, not the serial bridge).
- Reasons for the refused lease requests: not recorded in ccfarm's records.
