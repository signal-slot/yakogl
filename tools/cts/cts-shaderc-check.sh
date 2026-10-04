#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Is the host shader server built from a tree new enough to measure?
#
#   tools/cts/cts-shaderc-check.sh [required-commit]
#
# Prints the server's commit for the run header, and fails if the tree does not
# contain the required one.
#
# This exists because a stale server is invisible in the results. It answers
# every request, nothing errors, and the numbers describe a front end nobody is
# running any more -- a whole group can be measured, reported, and acted on
# before anyone notices. It has already happened once today in the other
# direction: an oracle run against a worktree that predated the fix nearly had a
# working change reported as broken.
#
# The check only works when the server runs on this host, which it does in this
# lab; elsewhere it says so and leaves the judgement to the operator rather than
# claiming a pass it cannot support.
set -uo pipefail

REQUIRED=${1:-}

pid=""
for d in /proc/[0-9]*; do
    cl=$(tr '\0' ' ' < "$d/cmdline" 2>/dev/null) || continue
    case "$cl" in
        *shaderc_server.py*)
            # The python process, not the shell that launched it.
            case "$cl" in *python*) pid=${d#/proc/}; break ;; esac
            ;;
    esac
done

if [ -z "$pid" ]; then
    echo "shaderc: no server process on this host." >&2
    echo "shaderc:   Either it runs elsewhere -- in which case check its commit by" >&2
    echo "shaderc:   hand -- or it is down, and every shaders.* case will fail with" >&2
    echo "shaderc:   'remote shaderc: cannot reach the compile server'." >&2
    exit 2
fi

cwd=$(readlink "/proc/$pid/cwd" 2>/dev/null)
[ -n "$cwd" ] || { echo "shaderc: cannot read the server's working directory" >&2; exit 2; }

head=$(git -C "$cwd" rev-parse --short HEAD 2>/dev/null)
subject=$(git -C "$cwd" log --oneline -1 2>/dev/null)
[ -n "$head" ] || { echo "shaderc: $cwd is not a git tree" >&2; exit 2; }

echo "shaderc: server pid $pid, tree $cwd"
echo "shaderc: commit $subject"

# The compiled tool matters as much as the tree: a rebuilt checkout with a stale
# binary answers with the old front end.
tool=$(ls -t "$cwd"/.target/*/yakogl-tools 2>/dev/null | head -1)
if [ -n "$tool" ]; then
    echo "shaderc: tool $tool ($(date -r "$tool" '+%Y-%m-%d %H:%M'))"
    src_newer=$(find "$cwd/tools" "$cwd/src" -newer "$tool" -name '*.rs' -print -quit 2>/dev/null)
    [ -n "$src_newer" ] && \
        echo "shaderc: WARNING: sources are newer than the tool binary; it needs rebuilding" >&2
fi

if [ -n "$REQUIRED" ]; then
    if git -C "$cwd" merge-base --is-ancestor "$REQUIRED" HEAD 2>/dev/null; then
        echo "shaderc: contains $REQUIRED -- ok"
    else
        echo "shaderc: DOES NOT contain $REQUIRED." >&2
        echo "shaderc:   The server would answer with an older front end and the run" >&2
        echo "shaderc:   would measure it. Ask shaderc-remote to rebuild and restart." >&2
        exit 1
    fi
fi
