<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# Redaction audit of the tree and the history (release stage 3)

Read-only audit for checklist line 8 (`docs/release/checklist.md`). Nothing
was edited, rewritten or deleted; this file is the only change.

Base: `5f8fff01` (ccfarm/integration tip, equal to `origin/main` in this
clone), audited on 2026-10-15.

## Verdict

**The history cannot be published as is**, neither the whole GitHub
repository nor `main` alone.

- **Every ref (what GitHub holds today):** one branch carries the PowerVR
  firmware image (finding H1). Other branches carry lab tailnet and LAN
  addresses (H4 to H6).
- **`main` alone:** it carries a tailnet address of the Pi 4 serial bridge
  and the Pi 4's lab login (H2, H3), both still in the tree. It also carries
  a farm node's tailnet address in an old file (H4), home paths, and 47
  versions of a ccfarm workspace file (M1, M2).
- **The tree at `5f8fff01`** needs edits before publication: H2, H3, M1,
  M3 and M4.

Recommended path: clean the tree with the edits listed below, then publish
it as a **fresh history** (one initial commit, or a squash at the release
point) in a **new** public repository, and keep the current one private.
This option is the cheapest one that is safe. Two reasons rule out
deleting branches and then flipping the current repository to public:

- GitHub keeps commits readable by SHA after their branch is deleted, and
  pull-request refs keep them too.
- `main` itself needs a rewrite anyway.

The alternative is a `git filter-repo` rewrite (section "History rewrite"),
which changes every commit id that the docs quote. The checklist's owner
decision "Publish rewritten history or a fresh history" is now due.

## Scope and method

| Item | Value |
|---|---|
| Refs scanned | all 496 refs of this clone: 139 `heads`, 355 remote-tracking (`origin/*`, i.e. what GitHub held at snapshot time), and the workspace branch and its base. `origin` itself was not reachable from this machine (ls-remote needs approval), so refs pushed after the snapshot are not covered |
| Commits | 1943 in all refs, 1240 reachable from `HEAD`. 287 remote-tracking branches are not merged into `HEAD`. `origin/ccfarm/integration` (`3b41437a`) has diverged from `HEAD`: it has 120 commits that `HEAD` does not |
| Blobs | all 6549 blobs in `git rev-list --all --objects`, text and binary. Five generated `blobs.rs` versions over 20 MB were scanned in a separate pass |
| Commit metadata | author and committer names and e-mails, and every commit message (same patterns) |
| Tree | `git ls-files` at `HEAD` with file:line. `third_party/` was scanned too, and its hits were triaged separately |
| Patterns | IPv4 (all, then triaged), IPv6 (link-local, ULA, global prefixes), `user@host`, `ssh/scp/rsync` targets, `/dev/tty*`, `/dev/serial/by-id/*`, `/home/<u>` and `/Users/<u>`, token shapes (`ghp_`, `github_pat_`, `sk-`, `AKIA`, `xox?-`, `AIza`, PEM private-key headers), `password/secret/api_key/token =` assignments, e-mail addresses, lab and worker host names, `pvr_fw_blob`, `rgx.fw`, `rgx.sh`, hex arrays over 2000 bytes, binary blobs, Imagination copyright lines with and without a licence grant, and "confidential/proprietary/NDA/internal use only" near TI, NXP, Vivante, Broadcom or Imagination |

**Masking.** Every value is masked as its first character, `***` and the
length, e.g. `s***[11]@e***[10]`. IPv4 addresses are given as their class
only, e.g. "tailnet (100.64/10)". Commit ids and file paths are not
secrets and are given in full.

## Findings

Severity: **H** = a project rule is broken (firmware, board address,
login) and the finding must be gone before publication. **M** = lab or
personal detail that should go. **L** = identifying but harmless; the
owner may accept it.

"Where" names the refs that hold the value. "main" means reachable from
`origin/main` = `HEAD`. "tree" means present in the files at `5f8fff01`.

### H1. PowerVR firmware image committed (class: firmware blob)

| Where | Detail | Action |
|---|---|---|
| commit `2584915e` (2026-10-08, ccfarm worker, "AM62P board session: per-case dEQP-GLES3 failure list … (turn 2)"), path `rgx.fw.36.53.104.796` at the repository root, blob `c9561b04`, 229376 bytes | The blob is a 32-bit ELF for machine 0xF3 (RISC-V), the BXS-4-64 firmware processor. Its name is the DDK's `RGX_FW_FILENAME "rgx.fw"` plus the BVNC. It is the Imagination firmware that the project rule says is never committed, and it is not covered by `LicenseRef-Imagination-DDK-MIT` | **History rewrite / ref removal.** Only one ref holds it: `origin/ccfarm/01a11ab1-df5e-7058-9bec-0b619ff3a607`. It is not on `main` or `ccfarm/integration`. Delete that branch on GitHub. Before any visibility change, also ask GitHub Support to purge the cached objects, or publish in a new repository (recommended). Check that no pull request references `2584915e` |

`driver/src/fw/pvr_fw_blob.h` was **never committed** in any ref: there is
no path matching `*pvr_fw_blob*` and no `driver/src/fw/` directory in any
commit. No hex array over 2000 bytes exists outside
`tests/reference/vivante_blobs.rs`, which holds GC7000 shader words made by
`tools/vivante/gen_shader_table.py` and is not firmware.

### H2. Tailnet address and port of the Pi 4 serial bridge (class: IPv4, host name)

| Where | Masked value | Action |
|---|---|---|
| tree: `docs/board-sessions/pi4-common-int.md:27-28` (bridge host `u***[4]`, tailnet (100.64/10) `1***[14]`, port `359**`), `:35` (host and port again) | tailnet IPv4 + bridge host + port | **Edit:** replace them with "the host:port in `$BOARD_UART`". The file belongs to the running item pi4-common-int, so the edit is left to that item or a later docs item |
| history: commit `76b3242d` (main, 2026-10-12) adds it; 12 blob versions of the same file in other refs (`fca07828`, `7c724433`, `1b952480`, `d20e174d`, `0d1285dd`, `86be8aeb`, …), in about 130 remote branches | same | **History rewrite** (or a fresh history) |

### H3. Pi 4 lab login (class: user@host)

| Where | Masked value | Action |
|---|---|---|
| tree: `docs/board-sessions/pi4-common-int.md:12,34,79`, `docs/board-sessions/pi4-plan1-int.md:12,13,49` | `s***[11]@e***[10]` | **Edit:** `"$BOARD_HOST"` or `<user@board>`. The first file is owned by the running pi4-common-int item |
| tree: lease/resource id `pi4-e***[8]` at `docs/es3-board-recipe-pi4.md:221,223,224`, `docs/board-sessions/pi4-plan1-int.md:22`, `docs/board-sessions/pi4-deqp-stage1-gles3.md:27`, `tools/cts/log/board-sessions/pi4-deqp-stage1-gles3.md:28` | the board's host name inside the id | **Edit:** `<pi4 resource>` |
| history: commits `76b3242d`, `9c2c893a`, `04ed90de` on main (2026-10-12), plus copies in other refs | same | **History rewrite** |

### H4. Tailnet address and port of a farm node (class: IPv4, host name)

| Where | Masked value | Action |
|---|---|---|
| history on main: `docs/farm/investigations/replica-zero.md`, added in `2e3da436` (2026-10-08) and removed from the tree in `1e2f0d9a` (2026-10-10). Blob versions `f91b2e35`, `90fab2f7`, `572ea045`; the file also names worker hosts and `/home/t***[6]` | tailnet (100.64/10) `1***[13]` with port `98**` | **History rewrite** for main |
| tree of `origin/ccfarm/integration` (`3b41437a`, on GitHub): the file is still there (blob `f91b2e35`), and so is `docs/farm/ccfarm-locations.md` (worker names) | same | That branch must not be published as it stands. Delete it from the public remote, or reset it to the cleaned `main` |

### H5. Console map with tailnet and LAN addresses (class: IPv4)

| Where | Masked value | Action |
|---|---|---|
| `docs/boards/console-map.md` (blobs `f28ffdaf`, `31ed10cd`), `docs/investigations/am62p-console-empty.md` (blobs `28506a2b`, `bccb68d8`, `e75427c7`), added in `638682b7` and `935f5fb8` (2026-10-09) | two tailnet (100.64/10) addresses `1***[15]` and `1***[14]` with ports `359**`, two LAN (192.168/16) addresses `1***[12]` | Not on main. They **are in the tree of `origin/ccfarm/integration`** and in 33 other remote branches. **Remove the refs** from the published remote, or rewrite. Main's copy, `tools/cts/log/investigations/console-map.md`, already uses `<tailnet address …>` placeholders |
| `docs/benchmarks-baseline/am62p.md`, blob `34d7125e` (commits `03fc61d3`, `fe12a7ed`) | tailnet `1***[15]` | one remote branch only (`origin/ccfarm/01a116f1-3165-7516-b00b-21a6b68412c0`): **delete the ref** |

### H6. Earlier, unmerged drafts of this audit carry the values in clear (class: IPv4, user@host, e-mail, serial)

| Where | Masked value | Action |
|---|---|---|
| `docs/release/redaction-audit.md` in commits `3a4e55f7` and `c33d3d40` (2026-10-07), blobs `a7841c10`, `d823eb07`, on 4 remote branches (`origin/ccfarm/01a1158b-c0f3-…`, `origin/ccfarm/01a115b1-7466-…`, `origin/ccfarm/01a115f0-60bd-…` and one more) | six LAN (192.168/16) addresses, two tailnet addresses, `t***[7]@<LAN address>`, e-mail `t***[6]@s***[14]` (not the public maintainer address), Debug Probe serial `E***` (partly masked) | **Delete those refs** from the published remote. The later draft `1b952480` (on `origin/ccfarm/01a13a41-1769-…`) masks its values but edits other files; this report supersedes it |

### M1. Home-directory paths (class: home path)

| Where | Masked value | Action |
|---|---|---|
| tree: `docs/board-sessions/imx8mp-deqp-stage1-gles3.md:21`, `docs/board-sessions/pi4-common-int.md:14`, `docs/board-sessions/pi4-deqp-stage1-gles3.md:9`, `docs/es3-zephyr-env.md:23`, `tools/cts/RESULTS.md:16868,16877`, `tools/cts/log/investigations/gc7000-femtovg-imx8mp.md:67-69`, `tools/precompile/measure-image.sh:45` (a script default) | `/home/t***[6]/…` (the owner's login) | **Edit:** use `$CCFARM_DEPS_DIR`, `$ZEPHYR_WS`, `$WEST_VENV` and `$ZEPHYR_SDK_INSTALL_DIR`. In the script, use `${ZEPHYR_WS:?}` |
| history: 226 blob versions in all refs (`tools/cts/RESULTS.md`, `.ccfarm-workspace.md`, `.ccfarm-scripts/*.py`, board-session docs, `docs/farm/…`, `tools/hostharness/glmark2/FINDINGS-*.md` and others). Also commit message `0f1bad11` (main) | same | **History rewrite** (`--replace-text`), or a fresh history |
| history, other refs only: `docs/farm/investigations/nuc-health-and-quota.md` (commits `f9e0264b` … `b1fdae97`, one remote branch `origin/ccfarm/01a11bcb-5310-…`) | `/home/s***[4]` (a second person's login) and a worker host name | **Delete the ref** |

### M2. Committed ccfarm worker files (class: home path, host name)

| Where | Detail | Action |
|---|---|---|
| history on main: `.ccfarm-workspace.md`, 47 commits (`1e0087f5` … `5abb436d`), 79 blob versions in all refs | ccfarm's workspace notes: `/home/t***[6]` paths, the node name `n***[3]`, the origin URL | **History rewrite:** `--path .ccfarm-workspace.md --invert-paths`, or a fresh history |
| history, other refs only: `.ccfarm-scripts/{extract2,extract_all,fetch2,fetch_all,fix_zephyr_git,rowD,setup_ws,stage1}.py` | `/home/t***[6]` paths | delete the refs or rewrite |
| history, other refs only: `CLAUDE.md` (`2f1ddf47`) | agent instructions; no address, path or credential found in it | delete the ref, or keep it if the owner agrees |
| tree: `.ccfarm/project.toml` (worker names at :170,:179; it also documents the board-address rule), history `.ccfarm/CYCLE.md` | farm configuration, no address | **Owner decision:** keep it or drop it from the published tree |

### M3. Module serial in a board host name (class: host name)

| Where | Masked value | Action |
|---|---|---|
| tree: `tools/cts/log/investigations/console-map.md:34,36`, `tools/cts/log/board-sessions/am62p-pr133.md:35` | `v***[12]-0864****` (an AM62P SoM's serial, from a login prompt) | **Edit:** `-<serial>`. Other places already use the zeroed form `-00000000` |
| history on main: `18d29afc`, `75f8407b` (2026-10-12) | same | rewrite, or accept (low value to an attacker) |

### M4. Serial bridge host names and ports (class: host name, UART path)

| Where | Masked value | Action |
|---|---|---|
| tree: bridge host `u***[4]` in `docs/es2-cts-failures.md:285`, `tools/cts/STATUS.md:18`, `tools/cts/RESULTS.md:1008,9641,12702,17204`, `tools/cts/log/handoff-2026-10-07-to-10-11.md:63`, `tools/cts/log/investigations/{console-map.md,am62p-console-map.md,integration-main-overlap.md}`; bridge host `u***-s***-1` in `docs/board-sessions/am62p-es2-final.md:5,25,34,37`, `docs/boards/am62p-console-lease.md:9,41,49`, `tools/cts/RESULTS.md` (14 lines, 1008 … 16079), `tools/cts/log/**` (about 20 lines) | TCP serial-bridge host names on the lab tailnet | **Edit:** "the bridge in `$BOARD_UART`". Most of these files are records (`tools/cts/log/**`, `RESULTS.md`), so a bulk replace by a docs item is enough |
| tree: ports `3593*` (four ports) in `tools/cts/RESULTS.md` (16 lines), `tools/cts/log/benchmarks-baseline/am62p.md:8,13,15,31`, `tools/cts/log/board-sessions/{am62p-pr133-baseline,am62p-pr133,am62p-gles3-percase-2026-10-13,farm-console-blocked-2026-10}.md`, `tools/cts/log/investigations/{console-map,am62p-console-map,pr133-report-draft,gc7000-table-pr129-merge-gate}.md`. `tools/board_selftest.sh:161-167` uses one port as fixture data | bridge ports | edit with the line above; the self-test fixture may stay |
| history: the same lines in earlier versions | same | covered by the rewrite or the fresh history |

### L1. Worker host names in commit metadata and messages (class: host name)

| Where | Detail | Action |
|---|---|---|
| author/committer of about 2000 commit fields: `ccfarm (<host>) <ccfarm@<host>>` for six worker hosts (`L***[6]`, `s***[8]`, `X***[9]`, `n***[3]`, `h***[4]`, `L***[12]`) | farm machine names | **Owner decision.** With a rewrite, use `--mailmap` to map them to one `ccfarm <noreply@…>`; a fresh history removes them |
| commit messages: worker names in about 75 messages (60 on main); "tailscale" in 3 (`f2a13877`, `28fb55ae` on main); `/dev/ttyACM9` (fixture) and `/dev/ttyS2` (board-side Linux name) | no address | accept, or drop with the history |
| tree: worker and runner names in `docs/ci.md`, `.github/workflows/gate.yml:54-64`, `tools/check.sh:309,313,2301`, `tools/setup-gate-host.sh:89,207`, `tools/cts/STATUS.md:13,26`, `docs/es3-zephyr-env.md`, `tools/cts/log/zephyr-env-lepton-2026-10-08.md` (two file paths, this one and `nuc-health-and-quota.md` in M1, contain a worker name; paths are shown in full) | self-hosted runner labels and the documentation of where the gate runs | accept (they document the CI), or generalise to "the gate host" |

### L2. Default board logins and product host names (class: user@host)

| Where | Masked value | Action |
|---|---|---|
| tree: `tools/cts/RESULTS.md:15875-15881`, `tools/cts/log/board-sessions/imx8mp-common-int.md:11,40`, `tools/cts/log/benchmarks-baseline/am62p.md:12,31`, `tools/cts/RESULTS.md:15636` and `tools/cts/log/x2-direct-boot.md:218-224` (`r***[4]@a***[15]` and `root@BOARD`) | the vendor images' default user and mDNS names (`t***[7]@v***[13]`, `t***[7]@v***[12]`, `r***[4]@a***[15]`) | low risk: they are product defaults, not lab addresses. Replace them with `$BOARD_HOST` if no lab name should remain |
| tree: `/home/t***[7]` (the board's default user home) in `tools/cts/RESULTS.md:5376,5475,5555,5736,5833,8405,15878,15879` and `driver/tools/kexec-zephyr/board-sudo-setup.sh:72,73` | on-board paths | no action (overridable defaults on the board) |

### L3. Serial device paths (class: UART path)

| Where | Detail | Action |
|---|---|---|
| tree: `/dev/ttyS2`, `/dev/ttyS3` and others (board-side Linux names) in AM62P session docs; `/dev/ttyUSB0`, `/dev/ttyACM1`, `/dev/ttymxc1` in `tools/cts/RESULTS.md` and `tools/cts/log/x2-direct-boot.md`; `/dev/ttyACM0` in `docs/board-sessions/pi4-common-int.md:20` | generic kernel device names; no lab machine can be told from them | no action |
| tree: `/dev/serial/by-id/usb-Raspberry_Pi_Debug_Probe__CMSIS-DAP__<serial>-if01` at `docs/board-sessions/pi4-plan1-int.md:50`, `docs/es3-board-recipe-pi4.md:33`, `tools/cts/README.md:553` | placeholder only; no ref in history holds the real serial except the partly masked one in H6 | no action |

## Checked and clean

| Class | Result |
|---|---|
| Tokens, keys | No PEM private key and no `ghp_`/`github_pat_`/`gho_`/`AKIA`/`xox?-`/`AIza` value in any blob or message. The two `sk-` hits are a panel overlay name in `tools/cts/log/investigations/pvr-bringup/am62p-desk-research.md` (and its earlier path `driver/docs/gpu-recipes/research.md`) |
| Passwords | Only false positives: "sudo: a password is required" log lines in `tools/cts/RESULTS.md`, the sudoers `NOPASSWD:` line in `driver/tools/kexec-zephyr/board-sudo-setup.sh`, and `secret` variables in `third_party/mesa-26.2.1/src/util/xxhash.h` |
| IPv6 | none |
| Other IPv4 | `192.0.2.0/24` (RFC 5737 documentation range) and `10.11.12.*`/`172.16.0.9` fixtures in `tools/board.sh`, `tools/board_selftest.sh` and `tools/testdata/*.uart.ts` (the redaction self-test checks that they are masked); `127.0.0.1`; and dotted numbers that are not addresses: the BVNC `36.53.104.796`, TRM section numbers in `driver/rust/src/ip/*.rs` and `driver/docs/*-cold.md`, and version strings |
| E-mail addresses | In commits: the maintainer (`tasuku.suzuki@…`, GitHub noreply), GitHub's `noreply`, the bot and `noreply@anthropic.com`. In the tree: upstream authors in vendored headers (`driver/src/vivante/hw/*.xml.h`, `driver/src/v3d/v3d_tex.*`, `driver/samples/zephyr/slint/nostd/slint-nostd.patch`, `third_party/`), and placeholders on `.invalid` (`SECURITY.md`, `tools/check.sh:2120`). `.inputs/nostd/*/Cargo.toml` (other refs only) has upstream crate authors. None is private, except the address in H6 |
| Large or generated blobs | `tests/reference/blobs.rs` and the older `src/shaderc/blobs.rs` (35-38 MB) were made by Mesa's open `pco_shc` and glslang; no path, address or marking in them. `tools/cts/RESULTS.md` (about 1.4 MB per version) is covered above. Binary blobs: two PNGs, website images, `.inputs/*.crate` (open-source crates, other refs) and H1 |
| Imagination DDK material | `third_party/ti-img-rogue-ddk-25.2/` (50 files, one commit, `afb16335`) comes from TI's public `ti-img-rogue-driver` (`UPSTREAM`). 45 files carry the DDK's "Dual MIT/GPLv2" or "MIT" grant. The rest are `MIT-COPYING`, `GPL-COPYING`, `README`, `UPSTREAM` and the generated `config/am62p_linux_release/config_kernel.h` with its README. All of it is covered by `LICENSES/LicenseRef-Imagination-DDK-MIT.txt`. No other DDK file exists in any ref. Files with an Imagination copyright outside `third_party/` (`driver/src/pvr_*`, `driver/rust/src/gpu/pvr/render.rs`, `src/shaderc/online/powervr/*`, `tests/reference/pvr_internal.rs`, `tools/pco_shc/*`) are Mesa-derived and carry `Apache-2.0 AND MIT` |
| TI / NXP confidential markings | none. "internal use only" in `third_party/ti-img-rogue-ddk-25.2/include/pvrsrv_memallocflags.h:293,386` describes a flag in the upstream MIT file. "TI's proprietary" in `am62p-desk-research.md:31` describes TI's public binary stack and quotes its public build guide |
| `records/` | gitignored; no `records/` path in any commit of any ref |

## History rewrite (if the owner keeps the history)

Not run. If a rewrite is chosen instead of a fresh history:

1. Edit the tree first (H2, H3, M1, M3, M4) and land it on `main`.
2. Run `git filter-repo` on a fresh mirror:
   - drop `.ccfarm-workspace.md`, `.ccfarm-scripts/`, `.ccfarm/CYCLE.md`,
     `CLAUDE.md`, `docs/farm/`, `docs/boards/console-map.md`,
     `docs/investigations/am62p-console-empty.md` and
     `rgx.fw.36.53.104.796` (`--invert-paths`);
   - add `--replace-text` rules for the values in H2-H6, M1, M3 and M4.
     Keep the rules file outside the repository, because it contains the
     values;
   - add `--mailmap` for the worker identities (L1).
3. Push only `main` (and tags, if any) to a **new** repository. Push none of
   the `ccfarm/*`, `work/*`, `land/*` or `fix/*` branches.
4. Re-run this audit's scan over `git rev-list --all --objects` of the
   result.
5. Every commit id quoted in the docs (`docs/HANDOFF.md`,
   `tools/cts/RESULTS.md`, board-session headers, this file) then points
   to the old history. README already says the hashes predate publication.

## Notes for other items

- Licence (release-license-audit): `src/shaderc/online/powervr/cf.rs` says
  its construction is "transcribed" from Mesa `pco_cf.c` (MIT, Copyright
  Imagination), but it carries `SPDX-License-Identifier: Apache-2.0` only.
  The other transcribed pvr files use `Apache-2.0 AND MIT`.
  `driver/rust/src/gpu/pvr/freelist.rs` follows DDK functions and layouts
  under `Apache-2.0` only. Both should be checked.
  `third_party/ti-img-rogue-ddk-25.2/config/am62p_linux_release/config_kernel.h`
  has no SPDX header.
- A gate step that runs these patterns over `git ls-files` (and over the
  diff of a change) would stop new session logs from bringing the values
  back. It is not part of this item.
