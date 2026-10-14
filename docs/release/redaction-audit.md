<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Redaction audit before publication

Read-only audit of the working tree and of every blob reachable from `HEAD`
(1257 commits, 5086 blobs; the secret, token-prefix and tailnet-name
patterns were re-run over every blob with `git rev-list --objects HEAD`,
`git cat-file --batch` and `grep -a`, with no hit) for values that must not become public. The earlier audit
(`4b37a25`) never merged and is not reachable from this clone, so this one was
redone from scratch on the current tip.

Values are **masked** here: first character, `***`, and the length in
brackets, e.g. `t***[7]`. Placeholders such as `<user@board>` are not findings.
This file has to pass its own scan; nothing below is a real value.

## Scope and method

| Class | Pattern looked for |
| --- | --- |
| user@host | `name@host` in ssh/scp/`BOARD_HOST` context or with a known login name |
| address | IPv4 in 10/8, 172.16/12, 192.168/16, 100.64/10 (tailnet), 169.254/16; IPv6 full form and link-local |
| tailnet | `*.ts.net`, "tailnet" with a host name |
| serial | `/dev/serial/by-id/<value>` with a real (non-placeholder) serial |
| home path | `/home/<name>` and `/Users/<name>` (not `user`, `you`, `me`) |
| secret | private-key headers, `ghp_`/`github_pat_`/`AKIA`/`sk-`/`xox` tokens, `token`/`password`/`secret`/`api_key` assignments |
| firmware header | the generated PowerVR firmware header under `driver/src/fw/` |
| lab names | the farm's board and worker host names, lease ids, bridge host and ports |

Skipped: `third_party/` (upstream copyright e-mail addresses), `*.xml.h`,
`*.patch`, `tools/testdata/` (recorded fixtures). Binary blobs were not
scanned. Only the history reachable from `HEAD` was scanned, not the other
refs of the clone. Worker names were searched as whole words, case-insensitive
(`git grep -nwi`, and `git log -S` for history).

## Result in one view

| Class | Tree (HEAD) | History only | Verdict |
| --- | --- | --- | --- |
| firmware header | not tracked, ignored by `driver/.gitignore` | never committed (no object with that path in `rev-list --objects HEAD`; `git log --all -- <path>` is empty) | **clean** |
| private key / token / credential | none | none (see T4 for look-alikes) | **clean** |
| `/dev/serial/by-id` real serial | none (files show `<serial>` placeholders only) | none | **clean** |
| tailnet name (`*.ts.net`) | none | none | **clean** |
| home paths | 17 lines, 5 files remain (T1 rows marked open / no change); 3 doc files fixed by this item | 34 file/value pairs, 43 commits | fix tree docs, history owner decision |
| lab login `s***[11]@e***[10]` | fixed in `pi4-plan1-int.md` by this item (`pi4-common-int.md` was already clean) | 3 commits | history owner decision |
| tailnet IPv4 + bridge host/port | already clean in the tree | 1 commit | history owner decision |
| lab names (lease id, bridge host, ports, SoM host names) | 21 files | n/a | owner decision |
| worker host names (4 nodes) | 12 files, 86 lines (T5); 4 doc files cleaned by this item | 38 / 34 / 6 commits (H table) | owner decision |
| committed ccfarm worker files | `.ccfarm/project.toml` only | `.ccfarm-workspace.md`, `.ccfarm-scripts/*` | owner decision |

## Tree findings

### T1 Home paths (`/home/<user>`), masked as `/home/<t***[6]>` and `/home/<t***[7]>`

`t***[6]` is the owner's login, `t***[7]` is the board's default Toradex
login.

| File:line | Value | Fix | Status |
| --- | --- | --- | --- |
| `docs/board-sessions/imx8mp-deqp-stage1-gles3.md:21` | `/home/<t***[6]>/.local/share/ccfarm/deps/...` | `$CCFARM_DEPS_DIR` (already named in the line) | **applied** |
| `docs/board-sessions/pi4-deqp-stage1-gles3.md:9` | same | `$CCFARM_DEPS_DIR` | **applied** |
| `docs/es3-zephyr-env.md:23` | `/home/<t***[6]>/.local/share/ccfarm-local/deps/...` | `$CCFARM_DEPS_DIR` | **applied** |
| `docs/board-sessions/pi4-common-int.md` | was `/home/<t***[6]>/.local/share/ccfarm/deps/...` | `$CCFARM_DEPS_DIR` | already clean at the tip (rewritten by an earlier merge) |
| `tools/precompile/measure-image.sh:45` | default for `ZEPHYR_WS` is an owner checkout path | default to `${ZEPHYR_WS:?set ZEPHYR_WS}` or a path relative to `$CCFARM_DEPS_DIR` | open, script (not a doc) |
| `tools/cts/log/investigations/gc7000-femtovg-imx8mp.md:67-69` | `ZEPHYR_WS`, `WEST_VENV`, SDK dir under `/home/<t***[6]>/org/...` | `$ZEPHYR_WS`, `$WEST_VENV`, `$ZEPHYR_SDK_INSTALL_DIR` | open, outside this item's file set |
| `tools/cts/RESULTS.md:16868,16877` | `/home/<t***[6]>/org/yakogl-cts/...` | `$CTS_OUT`-style variable name | open, outside this item's file set |
| `tools/cts/RESULTS.md:5376,5475,5555,5736,5833,8405,15878,15879` | `/home/<t***[7]>/{gcgpu,kexec,vendor-probe,safeui}/...` (board's default login home) | low risk: product default; replace with `<board home>` if the owner wants none | open, owner decision |
| `driver/tools/kexec-zephyr/board-sudo-setup.sh:72,73` | `KO_DIR=/home/<t***[7]>/gcgpu`, `KEXEC_DIR=/home/<t***[7]>/kexec` | code default on the board; keep (it is the on-board path, overridable by `KO_DIR`/`KEXEC_DIR`) | no change |

Agent-written `docs/board-sessions/*.md` and `docs/benchmarks-baseline/*.md`
at the tip hold no `/home/<name>` worker path any more (re-checked with
`git grep -n '/home/'` on both directories); the older versions are in
history (H table).

### T2 Lab login and bridge (`docs/board-sessions/`)

| File:line (before this item) | Value | Fix | Status |
| --- | --- | --- | --- |
| `pi4-plan1-int.md:12,13,49` | `s***[11]@e***[10]` (lab login of the Pi 4) | `"$BOARD_HOST"` / `<user@board>` | **applied** |
| `pi4-common-int.md` | same login, plus tailnet host `u***[4]`, tailnet IPv4 `1***[14]` (CGNAT range), TCP port `359**` | `"$BOARD_HOST"`, "host and port as in `$BOARD_UART`" | already clean at the tip |

### T3 Lab and board names still in the tree (owner decision)

These identify the lab's hardware (lease ids, bridge host, ports, SoM host
names that include a module serial). None is a credential, but each tells a
reader where the lab's gear is.

| Class | Masked value | Files |
| --- | --- | --- |
| Pi 4 lease / resource id | `pi4-e***[7]` | `docs/es3-board-recipe-pi4.md:221-224`, `docs/board-sessions/pi4-deqp-stage1-gles3.md:27`, `docs/board-sessions/pi4-plan1-int.md:22`, `tools/cts/log/board-sessions/pi4-deqp-stage1-gles3.md:28` |
| serial bridge host | `u***[4]`, `usb-s***[8]-1` | 21 files; most lines in `tools/cts/RESULTS.md` (18), `tools/cts/log/investigations/console-map.md` (12) |
| bridge ports | `3593*` (three ports) | `tools/cts/log/benchmarks-baseline/am62p.md:31`, `tools/cts/log/board-sessions/farm-console-blocked-2026-10.md:26`, `tools/cts/log/investigations/*console*` |
| board host login | `t***[7]@v***[12]`, `t***[7]@v***[13]` | `tools/cts/log/benchmarks-baseline/am62p.md:12,31`, `tools/cts/log/board-sessions/imx8mp-common-int.md:11,40`, `tools/cts/RESULTS.md:15875-15881` |
| SoM host name with serial suffix `v***[N]-0864****` | 8-hex module serial | `driver/docs/am62p-display-dump-linux.txt:11`, `tools/cts/RESULTS.md` (3), `tools/cts/log/board-sessions/am62p-pr133.md:35`, `tools/cts/log/board-sessions/imx8mp-gc-bringup.md:155`, `tools/cts/log/investigations/console-map.md:34,36` |
| root login on dev board | `r***[4]@a***[15]`, `r***[4]@l***[9]`, `r***[4]@B***[5]` | `tools/cts/RESULTS.md:15636`, `tools/cts/log/x2-direct-boot.md:218-224`, `docs/direct-boot.md` |
| `.ccfarm/project.toml:195` | placeholder `<user@host>` only | none needed |

Proposed fix for the whole class: replace with `$BOARD_HOST`, `$BOARD_UART`,
`<board host>` and drop module serial suffixes (`...-0864****` becomes
`-<serial>`). `tools/cts/RESULTS.md` and `tools/cts/log/**` are another
item's files, so no edit was made here.

### T5 Worker host names (ccfarm nodes) still in the tree

Four worker nodes appear: `l***[6]` (also `L***[11]`, its local-model twin),
`s***[8]`, `h***[4]`, `n***[3]` (the owner's checkout machine), and one
laptop name `X***[8]`. These are machine names, not credentials, but they
name the farm. Counts are lines matched by
`git grep -nwi` on the four names (the fifth, `X***[8]`, matches only in
`docs/farm/devices.md`).

| File:line | Masked value | Context | Proposed fix |
| --- | --- | --- | --- |
| `.ccfarm/project.toml:170,179` | `s***[8]`, `l***[6]` | comments on measurements | owner decision (neutral: "a farm node") |
| `.github/workflows/gate.yml:54,55,64` | `h***[4]`, `n***[3]`, `s***[8]` | runner list in a comment | owner decision; runner names show in Actions logs anyway |
| `tools/check.sh:309,313,2301` | `h***[4]`, `l***[6]`, `n***[3]` | measurement comments | replace with "a 16-thread host" etc., owner decision |
| `tools/setup-gate-host.sh:89,207` | `n***[3]`, `s***[8]` | history comments | same |
| `docs/ci.md` (19 lines, e.g. 13,18-21,32-34,299-332) | `h***[4]`, `s***[8]`, `n***[3]` | CI timing report, includes a hardware model name for `h***[4]` | owner decision; the measurements stay valid with "runner A/B/C" |
| `docs/farm/devices.md` (44 lines) | all five names | the farm device report itself; its subject is the worker nodes | owner decision: drop the file from the release or keep it internal |
| `tools/cts/RESULTS.md:16979,17120`, `tools/cts/STATUS.md:13,26` | `n***[3]` | "run on the owner's machine" | replace with "the lead's machine" |
| `tools/cts/log/board-sessions/pi4-deqp-stage1-gles3.md:32`, `pi4-pr133-baseline.md:69`, `investigations/pr133-texel-levels.md:295` | `l***[6]` (in a file name), `n***[3]` | prose | owner decision |
| `tools/cts/log/zephyr-env-<l***[6]>-2026-10-08.md` (6 lines; the name is in the file name too) | `l***[6]` | copy of `docs/es3-zephyr-env.md` | rename and mask with the owner's other log moves |

Applied by this item (doc files it touched, neutral wording "a farm node" /
"the worker node"): `docs/es3-zephyr-env.md` (five places),
`docs/board-sessions/pi4-deqp-stage1-gles3.md` (two places),
`docs/board-sessions/pi4-plan1-int.md:52`. The earlier Pi 4 session notes
`pi4-common-int.md` had none left. Everything else in this table is left as
an **owner decision**: the names are the farm's own and some files report
measurements per machine.

### T4 Look-alikes that are not findings

| File:line | Why |
| --- | --- |
| `tools/board_selftest.sh:422-428,590,598` | recorded fixtures for the redaction self-test: RFC 1918 addresses `1***`, a locally administered MAC, a made-up `token=` value; the test checks that they are masked |
| `tools/check.sh:2120` | `t***[7]@g***[12]` uses the reserved `.invalid` TLD |
| `driver/tools/kexec-zephyr/board-sudo-setup.sh:188,200` | `PASSWD`-style match is the sudoers line `NOPASSWD:` (no secret) |
| `SECURITY.md`, `docs/release/readme-walkthrough.md` | `<security contact placeholder>` placeholder, a TODO for the owner |
| `.github/workflows/*.yml` | `actions/...@vN` action pins |
| `tools/cts/log/board-sessions/pi4-v3d-bringup.md:390` | version string `1***[6]` (QEMU), not an address |

## History findings (reachable from `HEAD`)

Only paths whose blob differs from the one at `HEAD` are listed; "blobs" is
the number of distinct historic versions that hold the value.

| Class | Path | Masked value | Blobs | Commits that introduced it (sample) |
| --- | --- | --- | --- | --- |
| home path | `.ccfarm-workspace.md` | `/home/<t***[6]>` | 43 | `ce25451`, `524836b`, `8d0bb13`, `1e0087f` (+39) |
| home path | `.ccfarm-scripts/{extract2,extract_all,fetch2,fetch_all,fix_zephyr_git,rowD,setup_ws,stage1}.py` | `/home/<t***[6]>` | 1 each | same group (2026-10-07) |
| user@host | `.ccfarm-scripts/{fix_zephyr_git,setup_ws}.py` | `c***[6]@i***[7]` | 1 each | same group |
| home path | `tools/cts/RESULTS.md` | `/home/<t***[6]>`, `/home/<t***[7]>` | 27 + 39 | many |
| home path | `docs/board-sessions/{am62p,imx8mp,pi4}-deqp-stage1.md`, `zephyr-ws-setup.md`, `imx8mp-gc-case10.md`, `imx8mp-common-int.md`, `pi4-es2-r1.md`, `am62p-plan1.md`, `pi4-common-int.md` | `/home/<t***[6]>` | 1-6 each | board-session merges |
| home path | `docs/benchmarks-baseline/imx8mp.md` | `/home/<t***[6]>` | 1 | one commit |
| home path | `docs/es3-host-env.md`, `docs/es3-host-toolchain.md`, `docs/investigations/es2-host-deqp-{baseline,v3d}.md`, `docs/farm/investigations/replica-zero.md`, `tools/hostharness/glmark2/FINDINGS-*.md`, `tools/precompile/measure-image.sh` | `/home/<t***[6]>` | 1-8 | |
| lab login | `docs/board-sessions/pi4-common-int.md` (also `pi4-plan1-int.md`) | `s***[11]@e***[10]` | 3 commits: `76b3242`, `9c2c893`, `04ed90d` | `e393fe6`, `afb1633` add the login form |
| tailnet IPv4 | `docs/board-sessions/pi4-common-int.md` | `1***[14]` | 1 | `76b3242` |
| private IPv4 | `docs/farm/investigations/replica-zero.md`, `driver/docs/rpi4-v3d-backend-plan.md` | `1***[13]`, `1***[6]` | 3 / 2 | LAN addresses of the lab, owner to confirm |
| board login | `docs/benchmarks-baseline/imx8mp.md`, `docs/board-sessions/{am62p-common-int,am62p-plan1,imx8mp-common-int,imx8mp-gc-case10,imx8mp-gltest-final}.md`, `tools/cts/log/benchmarks-baseline/am62p.md`, `tools/cts/RESULTS.md` | `t***[7]@v***[12/13]` | 1-39 | |
| worker host name | `docs/ci.md`, `docs/farm/devices.md`, `docs/es3-zephyr-env.md`, `tools/check.sh`, `tools/setup-gate-host.sh`, `.github/workflows/gate.yml`, `.ccfarm/project.toml`, `tools/cts/**`, many board-session docs | `s***[8]` | 38 commits touch it (`git log -i -S`) | `ae81d86`, `83115f9`, `1cf74cf`, `3f0bcd7`, ... |
| worker host name | same group | `l***[6]` | 34 commits | `ee9404e`, `d043fa1`, `cb476f6`, ... |
| worker host name | `docs/ci.md`, `tools/check.sh`, `.github/workflows/gate.yml` | `h***[4]` | 6 commits | `ee9404e`, `a5f396e`, `d043fa1`, ... |
| worker host name | `docs/farm/devices.md` and others | `n***[3]`, `X***[8]` | too short or generic to count by pickaxe | owner decision |
| fixture addresses | `tools/board_selftest.sh` | RFC 1918 / MAC / token fixtures | 13-15 | (see T4) |
| `website/try-it.html` | `u***[4]@y***[10]` | placeholder `user@yakogl...` | 3 | none needed |

No private key, token, `ghp_`/`AKIA`/`sk-` value, tailnet DNS name or
`/dev/serial/by-id` serial exists anywhere in reachable history. The firmware
header was never committed: `git rev-list --objects HEAD` has no entry for it,
and `git log --all -- <path>` is empty.

## Owner decision: history

Nothing was rewritten. A rewrite changes every commit id since the first
touched commit (the repository has 1257 reachable commits and many branch and
ccfarm references to them), so it needs the owner. Options, cheapest first:

1. **Do not rewrite; publish a squashed tree.** Publish `HEAD`'s tree as a
   single initial commit (or a fresh history from the release point) and keep
   the current repository private. Removes every history finding at once.
   Recommended: the history contains 43 versions of ccfarm snapshot files
   and many lab-specific session logs that have no value publicly.
2. **Rewrite with `git filter-repo`** (not run here):
   - drop paths `.ccfarm-workspace.md`, `.ccfarm-scripts/`;
   - `--replace-text` rules for the home path of the owner and the board
     default login, the lab login, the bridge host and ports, the tailnet
     address, the worker host names and the SoM host-name serial suffix
     (rules file kept outside the repository, since it would itself contain
     the values);
   - `git filter-repo --path driver/src/fw/ --invert-paths` is not needed
     (never committed) but is cheap insurance.
3. **Accept** the home paths, default Toradex names and worker names as
   non-sensitive and rewrite only the lab login, tailnet address and
   `.ccfarm-*` files.

After any of these, force-push to a *new* public remote only; never to the
current one, and re-run this audit (`git grep`, `git rev-list --objects`) on
the result. Also decide the other refs (branches in this clone) separately:
this audit did not scan them.

## Applied in this item (tree, docs only)

- `docs/board-sessions/pi4-plan1-int.md`: lab login to
  `$BOARD_HOST`/`<user@board>`; worker name to "a farm node".
- `docs/board-sessions/imx8mp-deqp-stage1-gles3.md`,
  `pi4-deqp-stage1-gles3.md`, `docs/es3-zephyr-env.md`: owner home path
  dropped in favour of `$CCFARM_DEPS_DIR`; worker node names made neutral.

## Left for other items

- `tools/precompile/measure-image.sh:45` default path (script change).
- `tools/cts/RESULTS.md`, `tools/cts/log/**`, `docs/es2-cts-failures.md`,
  `tools/cts/STATUS.md`: lab names and home paths (T1, T3, T5).
- `docs/ci.md`, `docs/farm/devices.md`, `.github/workflows/gate.yml`,
  `tools/check.sh`, `tools/setup-gate-host.sh`, `.ccfarm/project.toml`:
  worker names (T5), owner decision.
- `docs/es3-board-recipe-pi4.md`: lease id `pi4-e***[7]` (T3).
- Add a gate step that runs these patterns over `git ls-files` (a small
  python scanner next to `tools/check.sh`), so new session logs cannot
  reintroduce the values.
