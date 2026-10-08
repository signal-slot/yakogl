# ccfarm locations (fact-finding, PARTIAL)

No addresses, logins or secrets are recorded here. Each fact carries its
origin: **confirmed here** (a command ran in the XPS139340 worker session),
**reviewer-reported** (stated by the automatic review, which ran in its own
session; not re-verified from this worker), or **unconfirmed**.

## Permission boundary of worker sessions

Observed in the XPS139340 worker session: `which ccfarm`, `readlink -f` and
`hostname` ran. `ccfarm --help` and `ccfarm --version` were refused ("requires
approval") on two attempts, `ask_farm` was refused ("not granted") on two
attempts, and `ls` of `~/.config/ccfarm` and `~/.local/share/ccfarm` was
blocked by the sandbox (only the task workspace and deps dir are listable).

Reviewer-reported policy: workers may run `ccfarm --help`, `ccfarm --version`,
`ask_farm`, `which`/`readlink` and the gate; the general `ccfarm:*` rule
(`home`, `status`, `peers`, ...) and `systemctl --user list-units` are denied
("internal task; not allowed"); the sandbox hard-blocks ccfarm's own
directories. The same review reports `--help`, `--version` and `ask_farm`
work on retry in its session, but they did not in this one, so the policy is
not uniform across sessions.

What later items can collect from a worker session: binary path and version
(if `--version` is permitted), `ccfarm --help` text, `ask_farm` answers,
`.ccfarm/project.toml` in the repo. What needs the owner or the nuc session:
anything under `~/.config/ccfarm` and `~/.local/share/ccfarm`, `ccfarm home`,
`status`, `peers`, journalctl and unit state.

## Per-node table

| Node | Binary | Version | Source checkout | Config | Data | Logs / state | Units |
|---|---|---|---|---|---|---|---|
| XPS139340 | `~/.cargo/bin/ccfarm`, not a symlink (confirmed here; cargo-install placement is inferred from the path, not confirmed) | 0.1.0, commit 742c008, built 2026-10-09 (confirmed here) | unconfirmed | unconfirmed | `~/.local/share/ccfarm` exists (confirmed here via workspace path; contents unreadable) | unconfirmed | unconfirmed |
| Lepton-local | `~/.cargo/bin/ccfarm`, cargo install (reviewer-reported) | 0.1.0, commit 742c008, built 2026-10-09 (reviewer-reported) | unconfirmed | default config plus `~/.config/ccfarm/local.toml` (reviewer-reported) | `~/.local/share/ccfarm-local` for the local instance (reviewer-reported) | unconfirmed | two daemons running: `ccfarm daemon` (default config) and `ccfarm --config ~/.config/ccfarm/local.toml daemon` (reviewer-reported); supervision mechanism unconfirmed |
| nuc | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed (home of the yakogl records, per the task brief) | unconfirmed | unconfirmed |
| system76 | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed |
| Lepton | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed |

The brief's claim that all nodes run build c5641e5 is contradicted by the
reviewer's report for Lepton-local (commit 742c008). Treat versions as
per-node and unequal until each node's `ccfarm --version` is recorded.

The ccfarm per-task workspace is `~/.local/share/ccfarm/workspaces/<agent id>`
and the project dependency cache is `~/.local/share/ccfarm/deps/<project>`
(`$CCFARM_DEPS_DIR`); both confirmed here on XPS139340.

## Unreachable / unanswered nodes

nuc, system76, Lepton, Lepton-local were not contacted over ssh. `ask_farm`,
once permitted, queried all five nodes (XPS139340, Lepton, Lepton-local, nuc,
system76) and returned no answers: no node has notes on where ccfarm's
binary, source, config, data or logs live, or how the daemon is supervised
(confirmed here). The ssh logins are therefore also unknown; none were
guessed.

## Subcommands and config keys

Confirmed here from `ccfarm --help` (top level only; `ccfarm <sub> --help`
was refused for home, update, quota, pause, env, drain, so flags beyond those
in the top-level text are unknown). Subcommands relevant to this survey:

- `daemon` / `front`: `front` runs the front roles (MCP endpoint, Telegram,
  reports, reviewer, promoter, plans) in their own process next to `daemon`
  (`[front]`), so either can restart without the other.
- `drain`: finish running turns, claim no new work, exit (for upgrades;
  SIGHUP does the same).
- `update`: self-update from this node's source checkout's remote (`[update]`);
  no options shows the current state. Implies a per-node source checkout.
- `home`: `home` lists this node's roles and replicas; `home show <project>`
  shows what its replica holds; `home take <project>` makes this node the
  project's home now. The follower/replica mechanism.
- `peers`: peers known to the running daemon. `status`: nodes, quota, queues.
- `pause` / `resume [node]`: stop/start a node's worker claiming new work
  (running agents finish; queued work waits or goes to other nodes). No node
  means this one.
- `quota [node [project share|remove|fill|reset]]`: per-node, per-project
  shares; changes apply at once on that node and override its config file;
  `reset` returns to the config file.
- `env`: node-local project env values (`env_names` in `.ccfarm/project.toml`):
  `list`, `set <project|*> <NAME> <value|file:path>`, `unset`, `--node`.
- `recheck`: re-check results held as `needs_environment` now; ccfarm also does
  it at start and when tools appear (daemon picks it up within a minute).
- `ask`: the CLI form of `ask_farm`. `resources`, `mcp`, `mcp-add`, `gc`,
  `plans`/`plan`, `inbox`, `decide`, `report`, `consult`/`reply`, `stages`,
  `discover`, `chat`, `init` also exist.
- Global options: `--config <file>` (default `$CCFARM_CONFIG` or the platform
  config dir), `--instance <name>` (`<name>.toml` next to the main config).
- Config sections named in help: `[front]`, `[update]`, `[mcp]`
  (`manage_user_config`, default on), `[gc]`, `[comms]` (`chat_auth`),
  `[[resources]]`. Key values: unconfirmed (config files unreadable here).

Earlier reviewer-reported detail, unverified here:

- `home` (`home show <project>`, `home take <project>`): project homes, their
  roles and replicas. This is the follower/replica mechanism.
- `pause` / `resume`: node placement on/off.
- `quota`: per-node, per-project shares.
- `env`: node-local project env values.
- `peers`: peer nodes.
- `update`: self-update from this node's source checkout's remote (implies a
  per-node source checkout).
- `mcp-add`, with `[mcp] manage_user_config`: the daemon registers itself in
  Claude Code settings.
- `--config` (default `$CCFARM_CONFIG` or the platform config dir) and
  `--instance <name>` (`<name>.toml` next to the main config); `ccfarm init
  --role both` writes the local-model daemon.
- Config sections named in help: `[front]`, `[update]`, `[mcp]`, `[gc]`,
  `[comms]`, `[[resources]]`. Key names and values inside them: unconfirmed.
- Repo-side: `.ccfarm/project.toml` (`[review]`, `promote_to`).
- Matching MCP tools seen in this session: `set_node_paused`,
  `get_quota_allocations` / `set_quota_allocations`, `get_project_env` /
  `set_project_env`, `list_resources` / `set_resource`, `list_mcp_servers`.

## Knobs for later items

(a) Restricting record-reading agents to nodes that can read the records:
`ccfarm pause` / `resume` (or `set_node_paused`) per node; `ccfarm quota`
(per-node, per-project shares: a zero share on a node that has no replica
would keep that project's work off it); `ccfarm home show <project>` to see
which nodes hold records. Whether a label/requires placement option exists is
unconfirmed; check `ccfarm --help` for it.

(b) Controlling the follower pulling: `ccfarm home show <project>` for replica
state and `ccfarm home take <project>` to change the home (do not run it
without the owner; it moves the authoritative copy). Restart is by restarting
the daemon on the follower; how the daemon is supervised (systemd user unit or
other) is unconfirmed. Lepton-local shows two daemons selected by `--config`.

Read-only commands for someone with unrestricted access, per node:

```
command -v ccfarm; readlink -f "$(command -v ccfarm)"; ccfarm --version
ccfarm --help
ccfarm home
ls ~/.config/ccfarm ~/.local/share/ccfarm ~/.local/state/ccfarm
systemctl --user list-units 'ccfarm*'
journalctl --user -u 'ccfarm*' --since -1d | tail
```

## Gate note

`tools/check.sh --quick` on XPS139340 (run twice, same result) failed these
rows, then was killed at the 10-minute background limit with later rows unseen:
`clippy host-test,gpu-powervr` (deny `overly_complex_bool_expr` at
`src/gl/texture.rs:9199`); `clippy host-test,gpu-vivante,shaderc-online,no-shader-table`
(10 `nonminimal_bool` warnings); `clippy host-test,gpu-powervr,shaderc-online,no-shader-table`
(9 warnings); `clippy host-test,gpu-vivante,shaderc-online,test-knobs` (same
deny error). `cargo fmt --check`, the `yakogl-capi` no_std build and the
`host-test,gpu-powervr` tests passed. The review reports it passes
on its node (99 ok, 0 failed). Treat the failures as node-environment
(toolchain version on XPS139340), not caused by this markdown-only change.
