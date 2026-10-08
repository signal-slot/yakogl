# Why XPS139340 has applied 0 of yakogl's records

Plan #5, item `facts-replica`. Investigated on 2026-10-08 between 13:50Z and 14:20Z by a ccfarm worker running **on XPS139340, the follower node itself**. The ccfarm build was c5641e5 on every node.

**Nothing in ccfarm's config or records was modified.**
Every command below was read-only: `farm_status`, `ccfarm home`, `ccfarm list`, `ccfarm peers`, `journalctl`, and reads of files.
The only change is this file.

## Root cause (one line)

XPS139340 never starts a pull of yakogl, because its daemon **does not know it has a yakogl clone**.
The follower loop starts a pull only for projects with a known clone, and the same check makes XPS advertise `candidate: false`.
XPS's config has no `[comms] projects_roots` and no `telegram_projects`, so the only other way to find a clone is the repositories of tasks in XPS's own store.
That store has been empty since XPS handed its two yakogl rows to nuc on 2026-10-08T06:54:08Z.

Classification: **configuration**.
The follower role is registered, but pulling never starts.
It is not a connection, authentication or nuc-side fault.

## Confirmed

Each entry gives the command or path, the quoted output, and the time (UTC).

1. **The homes block now**, from `mcp__ccfarm__farm_status detail=true`, `generated_at 2026-10-08T13:57:18Z`, answered by XPS139340:
   ```
   {"degraded":true,"followers":[{"applied_seq":0,"candidate":false,"lag":157787,
     "last_pull_secs":null,"missing_objects":0,"node":"XPS139340","online":true}],
    "holding_rows":[],"home":"nuc","key":"github.com/signal-slot/yakogl",
    "other_homes":[],"project":"yakogl","seq":157787,"term":1}
   ```
   nuc's log grew from 151,915 entries (the planner's read at 13:14Z) to 157,787, while `applied_seq` stayed at 0.
2. **XPS's roles**, from `ccfarm home` (13:58Z):
   ```
   github.com/signal-slot/yakogl  follower (term 1, home nuc)
   github.com/signal-slot/yakogl  replica of nuc (term 0, seq 7078, updated 2026-10-07 12:30:33 UTC)
   ```
   `ccfarm home show yakogl` gives the replica's path: `/home/tasuku/.local/share/ccfarm/replicas/github.com_signal-slot_yakogl@nuc.db`.
   It holds 285 tasks and plans #1 and #2.
   So XPS *did* replicate nuc under the old term-0 scheme, and that copy stopped at 2026-10-07 12:30:33Z.
   The term-1 log (seq 157787) has never been pulled.
3. **XPS went to sleep just after that last update.**
   `journalctl --user -u ccfarm.service` at 2026-10-08T06:56:27Z shows:
   `sentinel: this node was asleep or stopped for 1107 min key=slept:202610080656`.
   1107 minutes before that is about 2026-10-07T12:29Z.
4. **The handover that made XPS a follower**, from the same journal:
   ```
   2026-10-08T06:53:39.092746Z INFO ccfarm_mcp::failover: another node is the project's home: stepping down key=github.com/signal-slot/yakogl home=nuc term=1
   2026-10-08T06:54:06.647944Z INFO ccfarm_mcp::failover::takeover: handing the project over: its rows are frozen here key=github.com/signal-slot/yakogl to="nuc" term=1 seq=0
   2026-10-08T06:54:08.833572Z INFO ccfarm_mcp::failover::takeover: handed the project over: following the new home key=github.com/signal-slot/yakogl to="nuc" tasks=2
   ```
   Earlier, `2026-10-07T09:46:01.595357Z INFO ccfarm_mcp::home: project keys recorded tasks=2` shows that XPS held exactly two yakogl rows of its own.
5. **No pull was ever logged.**
   `journalctl --user --since "2026-10-07 00:00" | grep -iE 'replic|follow|pull'` matches only the handover line above and the GC lines (`replica_refs=0`).
   It shows no error, no connection failure and no authentication failure.
   (A failing pull would only log at debug level, `follow.rs:100`, so this alone does not rule out failing pulls; items 9–10 do.)
6. **XPS's own task store is empty.**
   `ccfarm list --limit 1000` printed nothing (14:10Z).
7. **XPS's node config sets nothing about homes or clones.**
   `/home/tasuku/.config/ccfarm/config.toml` holds only `[mcp]`, `[worker]`, `[p2p]`, `[quota]` and `[gc]`.
   It has no `[home]` and no `[comms]` section, so `projects_roots = []` and `telegram_projects = {}`.
8. **XPS does have a clone; ccfarm just isn't told about it.**
   `/home/tasuku/com/github/signal-slot/yakogl/.git/config` has `[remote "origin"] url = git@github.com:signal-slot/yakogl.git`.
   That URL normalises to the key `github.com/signal-slot/yakogl`.
9. **The code path.**
   ccfarm's source checkout is at `/home/tasuku/com/github/signal-slot/ccfarm`; the self-update log line names it as `source=/home/tasuku/com/github/signal-slot/ccfarm`.
   - `crates/mcp/src/failover/mod.rs`, `sync_followers` (lines 872–908): a pull of home H is started only when `self.settings.candidate && has_clone(clones, key)` and H's hello advertises the key with `acting`.
   - `is_candidate` (lines 530–535) = `settings.candidate && has_clone(clones, key) && front running && !draining`, and this is the value XPS advertises as `candidate`.
   - `refresh_clones` (lines 458–489) builds `clones` from `[comms] projects_roots` (repositories directly below), `[comms] telegram_projects`, and the `repo_root` of the 200 most recent isolated tasks in **this node's store**.
     With an empty config and an empty store, `clones` is empty.
   - `crates/config/src/home.rs`: `[home]` defaults to `replicate = true, failover = true, candidate = true`, so `settings.candidate` is true on XPS.
   - XPS runs its front roles: `2026-10-08T11:53:08.140382Z INFO ccfarm::front_roles: front roles running node=XPS139340`.
   - XPS is not draining: it runs 10 turns.
   - So the only false term in `candidate` is `has_clone`, and the same term stops `sync_followers` from ever creating the `Follower`.
10. **Why `applied_seq` is 0.**
    `advert()` (mod.rs lines 821–843) reads `applied_seq` from the running `Follower` of `(key, home)`, and falls back to `(0, 0)` when there is none.
    0 here means "no follower task exists", not "pulled and got nothing".
11. **Why `last_pull_secs` is null.**
    `crates/mcp/src/failover/status.rs` lines 41–53 and 85 take it from the *answering node's* `homes().acks(key)`, which is the record of pulls that node served.
    When `farm_status` is answered by a non-home node, as here on XPS, it is null by construction.
    Only nuc's answer can show a pull time, and nuc would have had to receive one.
12. **Why `degraded` is true.**
    status.rs lines 101–107: a home is degraded unless some follower is online, has `lag < 50` and `candidate == true`.
    XPS fails on `lag` and on `candidate`.
13. **The P2P link is up.**
    `ccfarm peers` (13:58Z) shows `100.81.213.44:9877 nuc online … build c5641e5 (up to date)`.
    XPS runs nuc's tasks through it all day (`starting turn … source=peer 100.81.213.44:9877`), so neither connection nor authorisation is the cause.

## Unconfirmed

- **Whether nuc has ever received a pull from XPS.**
  This needs `farm_status` answered by nuc, or nuc's journal.
  Both were unreachable from here (see below).
  The code says XPS never sends one.
- **Whether `projects_roots` alone is enough**, or whether the term-1 snapshot then downloads cleanly.
  The snapshot is large (157k log entries; `follow.rs` takes a snapshot first) and could hit a size or time limit.
  This was not tested, because testing it means changing config.
- **Whether XPS would have found the clone before the handover**, through its two yakogl rows' `repo_root`.
  This is inferred from the term-0 replica having been updated until 2026-10-07 12:30:33Z.
  The rows are gone, so it cannot be checked now.
- **system76 and Lepton:** they don't appear as followers at all.
  The same cause (no `projects_roots`, no yakogl tasks in their own stores) is likely, but unchecked.
- The repeated `WARN ccfarm_peer::server: rejected P2P request addr=100.81.213.44:… reason="address is not a known peer"` lines at each XPS restart (for example 2026-10-08T11:53:07.927107Z) come from nuc's requests arriving before XPS's discovery has listed nuc, about 4 s after start.
  They look harmless, and they are unrelated: follower pulls go from XPS to nuc.

## Paths per node

| Node | Binary | Config | Data / records | Logs | Status here |
|------|--------|--------|----------------|------|-------------|
| XPS139340 | `/home/tasuku/.cargo/bin/ccfarm` (self-updates from `/home/tasuku/com/github/signal-slot/ccfarm`, `origin/main`) | `/home/tasuku/.config/ccfarm/config.toml` | `/home/tasuku/.local/share/ccfarm/` (store; replicas in `replicas/`; per-task logs in `logs/`) | `journalctl --user -u ccfarm.service` (systemd user unit `ccfarm.service`, `Restart` on; retained since 2026-10-05) | read |
| nuc | unknown | unknown (likely the same layout) | unknown; the home of yakogl's term-1 log | unknown | not reached |
| system76, Lepton, Lepton-local | not examined (outside this item) | | | | |

## Unreachable nodes and why

- **nuc:** the `ssh` permission was denied to this worker ("nobody decided in time"), so neither nuc's files nor its journal could be read.
  `ask_farm` was granted too late to be used for the login, and no login was guessed.
- **`systemctl --user list-units`** was denied.
  The unit name `ccfarm.service` comes from the journal (`ccfarm.service: Scheduled restart job, restart counter is at 10.` at 2026-10-08T11:53Z).

## Commands to start the follower pulling (for `replica-restart`)

On **XPS139340**, these are reversible.
Back up the config, add one key, and restart gracefully:

```sh
cp -a ~/.config/ccfarm/config.toml ~/.config/ccfarm/config.toml.bak-$(date -u +%Y%m%dT%H%MZ)
cat >> ~/.config/ccfarm/config.toml <<'EOF'

[comms]
# Clones directly below these directories are this node's projects: the
# home replicator follows them (has_clone), see docs/farm/investigations/replica-zero.md
projects_roots = ["~/com/github/signal-slot"]
EOF
ccfarm drain        # finishes running turns, exits; systemd restarts ccfarm.service with the new config
```

A narrower alternative is to name only yakogl, with `telegram_projects = { yakogl = "/home/tasuku/com/github/signal-slot/yakogl" }` under `[comms]`.
Both keys are also read by the Telegram front, which XPS doesn't configure.

To undo, restore the `.bak` file and run `ccfarm drain` again.

Check the result 1–2 minutes later (the clone scan runs every 60 s):

```sh
ccfarm home                     # expect a "replica of nuc (term 1, seq N…)" line with N rising
journalctl --user -u ccfarm.service --since -10min | grep -iE 'replic|follow'
```

Then run `farm_status detail=true`, on nuc if possible.
Expect XPS's `applied_seq` to rise toward `seq`, `candidate: true`, and `last_pull_secs` to become a number in nuc's answer.
`degraded` should turn false once `lag < 50`.

**Caveat:** once XPS is a candidate, the default `[home] failover = true` lets XPS **take over** yakogl automatically if nuc goes offline for 45 s (`failover_secs`).
That is the goal of plan #5, but the owner should know it is now live.
To follow without being electable, set `[home] candidate = false`.
That would also stop pulling, because `sync_followers` requires `settings.candidate`.
So the recommendation is to leave it true.
