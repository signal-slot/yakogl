# Why XPS139340 has applied 0 of yakogl's records

Plan #5, item `facts-replica`. Investigated on 2026-10-08 between 13:50Z and 14:05Z by a ccfarm worker running **on XPS139340, the follower node itself**.

**Outcome: root cause not established.**
The worker could not read any of ccfarm's config, data or logs, and had no ssh access to nuc.
Every probe outside the yakogl workspace was refused by the worker's permission policy (see "Unreachable" below).
This document lists what was confirmed, the best-supported candidate causes, and the exact commands that settle the question.
Whoever has the permissions can run those commands (the `replica-restart` item or the owner).

**Nothing in ccfarm's config or records was modified.**
The only change is this file.

## Confirmed

Each entry gives the command or path, the quoted output, and the time.

1. **The worker ran on the follower.**
   `hostname` → `XPS139340` (2026-10-08T13:53Z, `date -u` → `2026-10-08T13:53:20Z`).
2. **ccfarm binary on XPS139340.**
   `which ccfarm` → `/home/tasuku/.cargo/bin/ccfarm` (13:53Z).
   It was installed with `cargo install`, so the source is wherever that install pointed, which was not readable here.
3. **The worker could not inspect ccfarm itself.**
   Each of these was refused with "requires approval" or "blocked" (13:53Z–14:05Z):
   - `ccfarm --help` and `ccfarm help`
   - `ls` / Read of `~/.config/ccfarm`, `~/.config/ccfarm/config.toml` and `~/.local/share/ccfarm`.
     The refusal text was `ls in '/home/tasuku/.config/ccfarm' was blocked ... may only list files in the allowed working directories`.
   - `journalctl --user -u ccfarm*`, `systemctl --user list-units --all` and `ps -eo pid,lstart,args`
   - `ssh -o BatchMode=yes nuc true`, `git ls-remote origin HEAD`
   - the MCP tools `mcp__ccfarm__farm_status` (detail=true, twice) and `mcp__ccfarm__ask_farm`.

   So step (1) of the item, re-running farm_status with detail=true, **could not be done**.
   The only homes block available is the planner's copy below.
4. **No earlier note on this subject exists in the repository.**
   `git log --all --oneline -- docs/farm tools/farm` returned nothing, and `git log --all -i --grep=replica --grep=follower` returned nothing (14:00Z).

## Reported by the planner (not re-observed here)

The plan text quotes `farm_status` at **2026-10-08T13:14Z**:

```
homes[]: {project yakogl, key github.com/signal-slot/yakogl, home nuc,
          degraded true, seq 151915, term 1,
          followers [{node XPS139340, online true, applied_seq 0, lag 151915,
                      last_pull_secs null, candidate false, missing_objects 0}]}
```

The plan's goal text, written earlier, says "144,197 件中 0 件".
So nuc's `seq` grew by about 7,700 between the two reads while `applied_seq` stayed at 0.
That means nuc is still writing records, and the follower is still not pulling.

## Unconfirmed: candidate causes and what they predict

These candidates are ranked by fit with the data above.
None is confirmed, because no log line could be read.

| # | Candidate | Fits | Settled by (log/config signature) |
|---|-----------|------|-----------------------------------|
| A | **Follower pull never scheduled.** XPS is listed as a follower, but its daemon has no pull loop for this home. Possible reasons: replication is off in XPS's config, the project key differs (for example `github.com/signal-slot/yakogl` against `git@github.com:signal-slot/yakogl.git`), or the follower was registered on nuc's side only. | `last_pull_secs null`: a pull was never even attempted or recorded. `missing_objects 0`: nothing was ever compared. `online true` comes from the peer heartbeat, not from replication. | XPS journal has **no** replica, follow or pull lines at all; XPS config has no follower entry for yakogl, or its key string differs from nuc's. |
| B | **Pulls attempted but every one errors** (auth, TLS, an HTTP 4xx/5xx from nuc, or a schema/term mismatch). | This also gives `applied_seq 0`. It would only show `last_pull_secs null` if the field records successful pulls only. | Repeated error lines in the XPS journal naming nuc or yakogl. |
| C | **nuc not serving replication.** nuc is `degraded true` and quota-blocked until 2026-10-08T23:00Z, and its serving side may be gated on health. | This fits "degraded". It does not explain why the count was 0 even before nuc became degraded. | nuc journal shows the replication endpoint refusing or not started, or `degraded` gating it. |
| D | **Connection.** XPS cannot reach nuc's replication port, even though the peer heartbeat works. | Less likely: `online true` means some path works. It is still possible if replication uses a different port or transport. | Connect or timeout errors in the XPS journal. |

**Why `candidate` is false (inference, unverified):**
a follower that has applied 0 of 151,915 entries, with lag equal to the whole log, cannot take over as home without losing every record.
Any sane election rule would exclude it.
`candidate false` is therefore most likely a *consequence* of `applied_seq 0`, not a separate fault.
Check this against ccfarm's source or its `--help` text for the candidate rule (for example a maximum lag or "has ever pulled").

**Why `last_pull_secs` is null (inference, unverified):**
the field reports the seconds since the last pull.
`null` means the follower has no recorded pull at all, which fits A best.
Under B, it fits only if failed pulls are not recorded.

## Paths per node

| Node | Binary | Config | Data / records | Logs | Status here |
|------|--------|--------|----------------|------|-------------|
| XPS139340 | `/home/tasuku/.cargo/bin/ccfarm` (confirmed) | `~/.config/ccfarm/` (exists: the refusal named it; contents unread) | `~/.local/share/ccfarm/` (exists: it holds `workspaces/` and `deps/`; records location unread) | expected `journalctl --user` for the ccfarm unit, unread; `~/.local/state/ccfarm/` unchecked | unreadable from the worker |
| nuc | unknown | unknown (same layout expected) | unknown; the home of yakogl's records | unknown | not reached: ssh refused, login not obtained (ask_farm refused) |
| system76, Lepton, Lepton-local | not examined (out of scope for this item) | | | | |

## Unreachable nodes and why

- **nuc:** `ssh nuc` was refused by the worker's permission policy, and the ssh login could not be looked up because `ask_farm` was refused.
  No login was guessed.
- **XPS139340 (local) ccfarm state:** the worker's sandbox only allows the workspace and `~/.local/share/ccfarm/deps/yakogl`.

## Commands to find the cause and start the follower pulling

Run these as the owner, or in a session that has these permissions.
Steps 1–4 are read-only.
Step 5 changes daemon state but is reversible (a restart), and step 6 needs ccfarm's actual subcommand names from step 1.

On **XPS139340**:

```sh
# 1. What the CLI offers for homes/replication
ccfarm --help
ccfarm --help 2>&1 | grep -inE 'home|follow|replic|pull|sync|candidate'

# 2. Follower configuration and the project key it uses
ls -la ~/.config/ccfarm ~/.local/share/ccfarm ~/.local/state/ccfarm
grep -rniE 'follow|replic|home|pull|yakogl|signal-slot' ~/.config/ccfarm

# 3. Daemon unit and its replication log lines since the follower was registered
systemctl --user list-units --all --no-pager | grep -i ccfarm
journalctl --user -u '<ccfarm unit from above>' --since 2026-10-01 --no-pager \
  | grep -iE 'replic|follow|pull|home|yakogl|nuc|term|seq' | tail -200
```

On **nuc** (login from `mcp__ccfarm__ask_farm "ssh login of nuc"`):

```sh
# 4. Is nuc serving the home, and what key does it use?
ssh <nuc-login> 'grep -rniE "follow|replic|home|yakogl|signal-slot" ~/.config/ccfarm'
ssh <nuc-login> 'journalctl --user -u <ccfarm unit> --since 2026-10-01 --no-pager \
  | grep -iE "replic|follow|pull|XPS139340|degraded|yakogl" | tail -200'
```

Read the result against the table:

- no pull lines on XPS → **A**
- errors on XPS → **B** or **D**, depending on the text
- refusals or "degraded" gating on nuc → **C**
- different key strings in the two configs → **A** (key mismatch).

Then:

```sh
# 5. (A, D, or after fixing B's credentials) restart the follower's daemon so it re-reads config
systemctl --user restart '<ccfarm unit>'      # on XPS139340

# 6. Confirm it pulls: applied_seq must rise and last_pull_secs must become a number.
#    Use mcp__ccfarm__farm_status detail=true (homes block) twice, a few minutes apart.
```

If A turns out to be a missing or mismatched follower entry, the fix is a config edit.
Make it as an added line, keep a `.bak` copy of the file, and use the key string exactly as nuc shows it.
That edit belongs to the `replica-restart` item, not to this one.
