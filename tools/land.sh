#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
#
# land.sh <name> <tip> <message-file>: land one branch tip on origin/main.
#
#   tools/land.sh land153 0a2cd9cf build/msg-land153.txt
#
# Steps, each refusing with its own name:
#   1. a fresh fetch; the tip must be a commit;
#   2. a detached worktree at origin/main under .claude/worktrees/<name>
#      (it must not exist yet);
#   3. `git merge --no-ff <tip> -F <message-file>` (a FILE: `git merge -F -`
#      cannot read stdin), then assert `HEAD^2 == <tip>`: a merge that did
#      not take the tip is a refusal, not a landing;
#   4. tools/check.sh in that worktree (the gate that counts is the one run on
#      the exact commit pushed); the log is build/gate-<name>.log there;
#   5. on gate rc 0 only: push HEAD to main, fetch again, and PROVE the TIP
#      (never HEAD, whose ancestry is trivially true) is an ancestor of
#      origin/main.
# Exit: 0 landed; 1 gate red (no push); 2 pushed but not proven; 8 the merge
# did not take; 9 a precondition.
# Environment: YAKOGL_REPO (default: the repository this script lives in),
# HAL_NXP_DIR (passed to check.sh), PVR_FW_BLOB (a generated pvr_fw_blob.h to
# copy into the worktree, default: the repository's own, if present).
set -u
NAME=${1:?usage: land.sh <name> <tip> <message-file>}
TIP=${2:?usage: land.sh <name> <tip> <message-file>}
MSG=${3:?usage: land.sh <name> <tip> <message-file>}
REPO=${YAKOGL_REPO:-$(cd "$(dirname "$0")/.." && pwd -P)}
cd "$REPO" || { echo "$NAME: no repository at $REPO"; exit 9; }
[ -r "$MSG" ] || { echo "$NAME: message file $MSG is not readable"; exit 9; }
MSG=$(cd "$(dirname "$MSG")" && pwd -P)/$(basename "$MSG")
git fetch origin -q || { echo "$NAME: FETCH FAILED"; exit 9; }
TIPH=$(git rev-parse --verify "$TIP^{commit}" 2>/dev/null) || { echo "$NAME: NO SUCH TIP $TIP"; exit 9; }
W=$REPO/.claude/worktrees/$NAME
[ -e "$W" ] && { echo "$NAME: WORKTREE EXISTS $W"; exit 9; }
git worktree add --detach "$W" origin/main >/dev/null 2>&1 || { echo "$NAME: WORKTREE ADD FAILED"; exit 9; }
cd "$W" || exit 9
echo "$NAME: base $(git rev-parse HEAD) tip $TIPH"
mkdir -p build
if ! git merge --no-ff "$TIPH" -F "$MSG" > build/merge-$NAME.out 2>&1; then
	echo "$NAME: MERGE FAILED"; cat build/merge-$NAME.out; exit 8
fi
[ "$(git rev-parse HEAD^2 2>/dev/null)" = "$TIPH" ] || { echo "$NAME: MERGE DID NOT TAKE THE TIP (HEAD^2 != tip)"; exit 8; }
echo "$NAME: merge $(git rev-parse HEAD) (HEAD^2 == tip),$(git diff --shortstat HEAD^1 HEAD)"
BLOB=${PVR_FW_BLOB:-$REPO/driver/src/fw/pvr_fw_blob.h}
if [ -r "$BLOB" ]; then mkdir -p driver/src/fw && cp -n "$BLOB" driver/src/fw/; fi
tools/check.sh > build/gate-$NAME.log 2>&1; rc=$?
echo "$NAME: gate rc=$rc $(date '+%F %T') ENOSPC=$(grep -c ENOSPC build/gate-$NAME.log)"
sed 's/\x1b\[[0-9;]*m//g' build/gate-$NAME.log | grep -E 'check.sh:|FAIL|red' | tail -n 3
[ $rc -eq 0 ] || { echo "$NAME: GATE RED: no push (build/gate-$NAME.log)"; exit 1; }
git push origin HEAD:refs/heads/main > build/push-$NAME.log 2>&1; prc=$?
git fetch origin -q
if [ $prc -eq 0 ] && git merge-base --is-ancestor "$TIPH" origin/main && git merge-base --is-ancestor HEAD origin/main; then
	echo "$NAME: LANDED $(git rev-parse HEAD): tip $TIPH is an ancestor of origin/main $(git rev-parse origin/main)"
	# The landing worktree is scratch (its .target is about 27 GB): remove it once
	# the push is proven. Four of them filled /home to 96% on 2026-10-04.
	cd "$REPO" && git worktree remove --force "$W" && echo "$NAME: worktree removed"
	exit 0
fi
echo "$NAME: NOT LANDED (push rc=$prc)"; cat build/push-$NAME.log; exit 2
