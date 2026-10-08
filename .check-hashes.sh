#!/bin/bash
# Verify every hex token (7-40 chars) cited in docs/plan-status.md resolves
# as a git object in this clone. Prints UNRESOLVED for the ones that do not.
set -u
doc="docs/plan-status.md"
for h in $(grep -oE '[0-9a-f]{7,40}' "$doc" | sort -u); do
  if git cat-file -e "${h}^{commit}" 2>/dev/null; then
    echo "ok    $h"
  else
    echo "UNRESOLVED $h"
  fi
done
