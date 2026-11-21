#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Compare an imx95_scmi run's lines with expected.txt (Toradex's public SM
# configuration). Usage:
#   check.sh CONSOLE_LOG [EXPECTED]
# CONSOLE_LOG is the run's console (any other lines and CRs are ignored).
# Prints one line per expectation that does not hold, then
#   imx95_scmi check: N expected, M match, D differ, K missing; verdict V
# and exits 0 only when every expectation matched and the probe's verdict
# line is PASS. Exit 2 for a usage error.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
log=${1:-}
exp=${2:-$here/expected.txt}
[ -n "$log" ] && [ -r "$log" ] && [ -r "$exp" ] || { echo "usage: check.sh CONSOLE_LOG [EXPECTED]" >&2; exit 2; }
tr -d '\r' < "$log" | awk -F '\t' -v explist="$exp" '
    BEGIN {
        while ((getline l < explist) > 0) {
            if (l ~ /^[[:space:]]*(#|$)/) continue
            split(l, f, "\t")
            if (f[1] == "" || f[2] == "") { print "expected.txt: bad line: " l; bad = 1; continue }
            n++; key[n] = f[1]; want[n] = f[2]
        }
        verdict = "none"
    }
    /^imx95_scmi: / {
        s = substr($0, 13)
        if (s ~ /^PASS/) { verdict = "PASS"; next }
        if (s ~ /^FAIL/) { verdict = "FAIL"; next }
        i = index(s, " = ")
        if (i == 0) next
        k = substr(s, 1, i - 1); v = substr(s, i + 3)
        if (!(k in got)) got[k] = v
    }
    END {
        for (j = 1; j <= n; j++) {
            k = key[j]
            if (!(k in got)) { print "missing: " k; miss++; continue }
            if (got[k] ~ ("^(" want[j] ")$")) ok++
            else { print "differs: " k " = " got[k] " (want " want[j] ")"; diff++ }
        }
        printf "imx95_scmi check: %d expected, %d match, %d differ, %d missing; verdict %s\n", \
            n, ok, diff, miss, verdict
        exit (bad || diff || miss || verdict != "PASS") ? 1 : 0
    }'
