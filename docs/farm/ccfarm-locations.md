# ccfarm locations (fact-finding, INCOMPLETE)

Status: the survey could not be carried out. In the worker session every
command that touches ccfarm or its directories was refused, and the
`ask_farm` tool was not granted. Nothing below is guessed; everything not
listed under "Confirmed" is **unconfirmed**. No addresses, logins or secrets
are recorded here.

## Confirmed

| Node | Item | Value | Source |
|---|---|---|---|
| XPS139340 (this worker) | hostname | XPS139340 | `hostname` ran |
| XPS139340 | per-task workspaces | `~/.local/share/ccfarm/workspaces/<agent id>` | session working directory |
| XPS139340 | project dependency cache | `~/.local/share/ccfarm/deps/yakogl` (`$CCFARM_DEPS_DIR`) | session additional directory |
| XPS139340 | ccfarm data root | `~/.local/share/ccfarm` exists | implied by the two paths above |
| all | ccfarm build | c5641e5 | task brief (not re-verified) |
| nuc | home node of the yakogl records | nuc | task brief (not re-verified) |

## Per-node table

All cells unconfirmed unless stated above.

| Node | Binary | Source | Config | Data | Logs / state | Units |
|---|---|---|---|---|---|---|
| nuc | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed |
| system76 | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed |
| XPS139340 | unconfirmed | unconfirmed | unconfirmed | `~/.local/share/ccfarm` (confirmed to exist via workspace path) | unconfirmed | unconfirmed |
| Lepton | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed |
| Lepton-local | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed | unconfirmed |

## Unreachable nodes

nuc, system76, Lepton, Lepton-local were not contacted: ssh logins come from
`ask_farm`, which was denied, and logins must not be guessed.

## Subcommands and config keys

Not collected (`ccfarm --help` was refused). Names seen in the task brief and
tool list only, meaning unverified: `ccfarm env` (per-project node-local env
values, `set <project> <NAME> <value|file:path>`); MCP tools `set_node_paused`,
`get_quota_allocations` / `set_quota_allocations`, `get_project_env` /
`set_project_env`, `list_resources` / `set_resource`, `list_mcp_servers`;
repository file `.ccfarm/project.toml` (enables `[review]`, `promote_to`).
No node-level config keys for placement or replication were observed.

## Knobs for later items

Candidate mechanisms, all unverified until the commands below are run:

- (a) Restrict record-reading agents: `.ccfarm/project.toml` in the repo
  (project-level), `set_node_paused` (pause a node), quota allocations; any
  label/requires option must be looked for in `ccfarm spawn --help` and
  `ccfarm nodes --help`.
- (b) Follower control: look in `ccfarm --help` for follower/replica/sync/home
  subcommands, and check `systemctl --user list-units 'ccfarm*'` and
  `journalctl --user -u 'ccfarm*'` on XPS139340 and nuc.

Commands for someone with shell access on each node (read-only):

```
command -v ccfarm; readlink -f "$(command -v ccfarm)"
ccfarm --help
ls ~/.config/ccfarm ~/.local/share/ccfarm ~/.local/state/ccfarm
systemctl --user list-units 'ccfarm*'
journalctl --user -u 'ccfarm*' --since -1d | tail
```
