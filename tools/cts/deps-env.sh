#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# deps-env.sh -- the project's dependency layout, as environment lines.
#
# Everything the gate and the board tools need that is not in the repository
# lives in one directory per host, in connected's layout for this project:
# $CONNECTED_DEPS_DIR when connected set it (a farm agent's turn; it sets the
# old name CCFARM_DEPS_DIR too), else ~/.local/share/connected/deps/yakogl
# (~/.local/share/ccfarm/deps/yakogl on a machine without it: the gate images
# built before 2026-10-26). A host that keeps a tree elsewhere links
# it into that directory under the entry's path. The entries (names, paths,
# markers, how to build each, its downloads and packages) are in
# tools/cts/deps.toml, the one place; a change that adds a gate row needing an
# external tree adds its entry there in the same commit.
# .github/workflows/gate.yml and the board recipes read the names from here.
#
#   tools/cts/deps-env.sh [board]
#     Print NAME=value lines for `$GITHUB_ENV` or `export $(...)`: the gate's
#     entries, and with a board (am62p | imx8mp | rpi_4b) that board's too.
#   tools/cts/deps-env.sh --layout
#     Print the layout directory (for scripts that need a path in it).
#   tools/cts/deps-env.sh --check [board]
#     One "ok" or "missing" line per entry; exit 1 when an entry is
#     missing. Each entry is checked by a file that only a usable tree has
#     (the marker shown on the line), so an empty directory or a venv
#     without west is "missing", not "ok". A missing entry is reported,
#     never skipped: a gate run without it would be a gap, not a pass. This
#     is what every host and image is judged by.
#   tools/cts/deps-env.sh --build [--board <board>] NAME...
#   tools/cts/deps-env.sh --build-missing [board]
#     Make entries (NAME is the name or an alias): fetch the entry's inputs
#     into <layout>/downloads (sha256 checked), run its `build` with $DEPS set
#     to the layout, and check the marker afterwards. An entry that is
#     present is never touched; entries it `requires` are made first; an
#     entry without a build (the firmware header) is reported and fails the
#     run. For hosts and images, run on purpose: the gate never builds these.
#     Install the packages first (--packages).
#   tools/cts/deps-env.sh --packages <apt|portage|brew>
#     The distribution packages of all entries, one per line, for a host or
#     image recipe (tools/setup-gate-host.sh uses it).
#
# GLSLANG is glslangValidator on PATH (a distro package, tools/setup-gate-host.sh).
# Needs python3 >= 3.11 (tomllib), a gate requirement anyway.
set -euo pipefail
exec python3 -I "$(dirname "${BASH_SOURCE[0]}")/deps_env.py" "$@"
