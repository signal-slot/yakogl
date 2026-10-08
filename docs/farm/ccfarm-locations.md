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
| XPS139340 | `~/.cargo/bin/ccfarm`, not a symlink (confirmed here; cargo-install placement is inferred from the path, not confirmed) | unconfirmed (`--version` refused) | unconfirmed | unconfirmed | `~/.local/share/ccfarm` exists (confirmed here via workspace path; contents unreadable) | unconfirmed | unconfirmed |
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

nuc, system76, Lepton, Lepton-local were not contacted from this session:
ssh logins come from `ask_farm`, which was refused here. The reviewer reports
`ask_farm` queried all five nodes but none recorded where ccfarm lives, so
that answer source is empty even when permitted.

## Subcommands and config keys

None collected from this session (`--help` refused). Reviewer-reported from
`ccfarm --help`, unverified here:

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

`tools/check.sh --quick` failed on this node's clippy steps (texture.rs:9199
`overly_complex_bool_expr`, plus ten `nonminimal_bool` warnings in `src/gl` and
`src/shaderc`) and was killed at the time limit. The review reports it passes
on its node (99 ok, 0 failed). Treat the failures as node-environment
(toolchain version on XPS139340), not caused by this markdown-only change.
