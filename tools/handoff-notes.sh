#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# handoff-notes.sh -- the hand-over notes waiting to be folded into
# docs/HANDOFF.md (docs/handoff/README.md).
#
#   tools/handoff-notes.sh          list the notes, oldest first
#   tools/handoff-notes.sh --check  exit 1 while any note waits (the fold's
#                                   last step; a person's check before
#                                   calling HANDOFF.md current)
set -euo pipefail
cd "$(dirname "$0")/.."
notes=$(find docs/handoff -maxdepth 1 -name '*.md' ! -name README.md | sort)
case "${1-}" in
  "")
    [ -n "$notes" ] && printf '%s\n' "$notes"
    exit 0 ;;
  --check)
    if [ -n "$notes" ]; then
      echo "handoff-notes: notes waiting to be folded into docs/HANDOFF.md:" >&2
      printf '  %s\n' "$notes" >&2
      exit 1
    fi
    echo "handoff-notes: nothing waits"
    exit 0 ;;
  *)
    echo "usage: $0 [--check]" >&2
    exit 2 ;;
esac
