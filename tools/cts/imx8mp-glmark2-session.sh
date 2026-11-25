#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/imx8mp-glmark2-session.sh <NN|gmNN-label> [--dry-run]
#       [--record-dir DIR] [--ref-dir DIR]
#
# One i.MX8MP glmark2 scene from the frame dump to a checked verdict row. It
# stops at the first failing step with a non-zero exit and a line
# "session: step <name> failed: ..." on stderr. Steps:
#
#   1 resolve    the scene through `imx8mp-glmark2-dumps.sh scenes`
#   2 run        unless --record-dir names an existing record,
#                `imx8mp-glmark2-dumps.sh run <img-dir> <records-dir> <scene>`
#                (the board; BOARD_UART and the rest come from the
#                environment the dumps script reads, nothing here prints,
#                stores or takes them as an argument). With --dry-run the
#                command is printed and nothing runs, no board is touched.
#   3 decode     glmark2-fb.py on the record's console-live.txt (console.txt up
#                to a "board: record" line without it). Any damaged or missing
#                row: exit 3, "re-run, never patch"; nothing after it runs.
#   4 redaction, record-check
#                imx8mp-gles3-redact.py --check and
#                imx8mp-glmark2-record-check.py on the record, both exit 0.
#                record-check checks a whole 33-scene session, so for one
#                scene only its problems about this record count: the
#                "console: ... scene NN" lines of other scenes and every
#                "photograph:" line (a dump record has no photographs) are
#                left out; name, commit, console for this scene, sha256 and
#                redaction lines fail the step.
#   5 compare    glmark2-picdiff.py and glmark2-sidebyside.py against
#                <ref-dir>/<slug>.png (the llvmpipe reference of
#                glmark2-host-ref.sh) into the record: <label>.picdiff.png,
#                <label>.sidebyside.png.
#   6 judge      imx8mp-glmark2-judge.py's proposed row when the tool exists,
#                else the inputs for imx8mp-glmark2-note-fill.py. The exact
#                note-fill command is printed; writing the row into the note
#                is never automatic (a person looks at the pictures first).
#
# Environment: IMX8MP_IMG_DIR (the images of `imx8mp-glmark2-dumps.sh build`,
# required for step 2), IMX8MP_RECORDS (records directory, default
# records/imx8mp of the checkout). --ref-dir defaults to the newest
# records/host/*-gmref-* directory. Exit: 0 done, 1 a step failed, 2 bad
# argument or setup, 3 damaged or missing rows (re-run the scene).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
DUMPS=$HERE/imx8mp-glmark2-dumps.sh
NOTE=docs/notes/imx8mp-glmark2-pictures-by-eye.md

usage() { sed -n '/^#   tools/,/^#$/p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }
newest() { # newest of the paths given (names sort by date); empty when none exists
    local p last=
    for p in "$@"; do [ -e "$p" ] && last=$p; done
    printf '%s' "$last"
}
step_fail() { echo "session: step $1 failed: $2" >&2; exit "${3:-1}"; }

DRY=0
RECDIR=
REFDIR=
SCENE=
while [ $# -gt 0 ]; do
    case $1 in
    --dry-run) DRY=1 ;;
    --record-dir) [ $# -ge 2 ] || usage; RECDIR=$2; shift ;;
    --ref-dir) [ $# -ge 2 ] || usage; REFDIR=$2; shift ;;
    -h | --help) usage ;;
    -*) usage ;;
    *) [ -z "$SCENE" ] || usage; SCENE=$1 ;;
    esac
    shift
done
[ -n "$SCENE" ] || usage

# 1 resolve
scenes=$(bash "$DUMPS" scenes) || step_fail resolve "imx8mp-glmark2-dumps.sh scenes failed" 2
line=
if [[ $SCENE =~ ^[0-9]+$ ]]; then
    n=$((10#$SCENE))
    [ "$n" -ge 1 ] && line=$(printf '%s\n' "$scenes" | sed -n "${n}p")
else
    line=$(printf '%s\n' "$scenes" | awk -v s="$SCENE" '$1 == s { print; exit }')
fi
[ -n "$line" ] || step_fail resolve "no scene '$SCENE' (imx8mp-glmark2-dumps.sh scenes lists them)" 2
LABEL=${line%% *}
rest=${line#* }
INFO=${rest%% -b *}
ARG=${rest##* -b }
SCENE_NAME=${ARG%%:*}
slug=$(python3 -I -c 'import re, sys; print(re.sub(r"[^a-z0-9]+", "-", sys.argv[1].lower()).strip("-")[:48])' "$INFO")
echo "session: step resolve: $LABEL  $INFO  (-b $ARG)"

# 2 run
RECORDS=${IMX8MP_RECORDS:-$REPO/records/imx8mp}
runrc=0
if [ -n "$RECDIR" ]; then
    [ -d "$RECDIR" ] || step_fail run "--record-dir $RECDIR is not an existing record" 2
    echo "session: step run: skipped, record $RECDIR"
elif [ "$DRY" = 1 ]; then
    echo "session: step run (dry run, no board touched):"
    bash "$DUMPS" --dry-run run "${IMX8MP_IMG_DIR:-<img-dir>}" "$RECORDS" "$LABEL" || step_fail run "dumps --dry-run failed"
    echo "session: dry run: decode, redaction, record-check, compare and judge need the record"
    exit 0
else
    : "${IMX8MP_IMG_DIR:?set IMX8MP_IMG_DIR to the images of imx8mp-glmark2-dumps.sh build}"
    bash "$DUMPS" run "$IMX8MP_IMG_DIR" "$RECORDS" "$LABEL" || runrc=$?
    RECDIR=$(newest "$RECORDS"/*-"$LABEL"-*)
    [ -n "$RECDIR" ] || step_fail run "no record of $LABEL under the records directory (dumps run rc=$runrc)"
fi
RECDIR=${RECDIR%/}
if [ "$DRY" = 1 ]; then
    echo "session: dry run with a record: the board step is skipped, the checks below run on it"
fi

# 3 decode
live=$RECDIR/console-live.txt
if [ ! -s "$live" ]; then
    [ -s "$RECDIR/console.txt" ] || step_fail decode "no console.txt in the record"
    awk '/board: record/ { exit } { print }' "$RECDIR/console.txt" > "$live"
fi
png=$RECDIR/$LABEL.png
# the decoder's output names the record path: kept in a variable, never in the record
decout=$(python3 -I "$HERE/glmark2-fb.py" "$live" "$png" --scene "$SCENE_NAME" 2>&1)
if [ $? -ne 0 ]; then
    printf '%s\n' "$decout" | sed -n '1,60p' >&2
    [ -e "$png" ] && mv -f "$png" "$RECDIR/$LABEL.damaged.png"
    step_fail decode "damaged or missing rows in the dump: re-run, never patch ($LABEL)" 3
fi
[ "$runrc" = 0 ] || step_fail run "dumps run exited $runrc though the dump decodes"
printf '%s\n' "$decout" | sed -n '1,5p'
echo "session: step decode: ok"

# 4 redaction, record-check
python3 -I "$HERE/imx8mp-gles3-redact.py" --check "$RECDIR" || step_fail redaction "an address, login or serial path in the record (matches are never printed)"
rcout=$(python3 -I "$HERE/imx8mp-glmark2-record-check.py" "$RECDIR")
rcrc=$?
[ "$rcrc" -le 1 ] || step_fail record-check "record-check exited $rcrc"
nn=$(printf '%02d' "$((10#${LABEL:2:2}))")
rcbad=$(printf '%s\n' "$rcout" | awk -v nn="$nn" '
    /^record-check: / { next }
    /^photograph:/ { next }
    /^console: no glmark2-picture line for scene / { if ($7 != nn) next }
    { print }')
if [ -n "$rcbad" ]; then
    printf '%s\n' "$rcbad" >&2
    step_fail record-check "problems in the record"
fi
echo "session: step redaction, record-check: ok (other scenes' and photograph lines left out)"

# 5 compare
if [ -z "$REFDIR" ]; then
    REFDIR=$(newest "$REPO"/records/host/*-gmref-*)
fi
{ [ -n "$REFDIR" ] && [ -s "$REFDIR/$slug.png" ]; } || step_fail compare "no reference $slug.png (--ref-dir, glmark2-host-ref.sh)"
python3 -I "$HERE/glmark2-picdiff.py" "$png" "$REFDIR/$slug.png" "$RECDIR/$LABEL.picdiff.png" || step_fail compare "glmark2-picdiff.py failed"
python3 -I "$HERE/glmark2-sidebyside.py" "$png" "$REFDIR/$slug.png" "$RECDIR/$LABEL.sidebyside.png" \
    --label-board "I.MX8MP $LABEL" --label-ref LLVMPIPE || step_fail compare "glmark2-sidebyside.py failed"
echo "session: step compare: ok ($LABEL.picdiff.png, $LABEL.sidebyside.png in the record)"

# 6 judge / note-fill inputs
rbase=$(basename "$RECDIR")
sha8=${rbase##*-}
relrec=records/imx8mp/$rbase/
if [ -f "$HERE/imx8mp-glmark2-judge.py" ]; then
    echo "session: step judge: proposed row (the verdict stays for a person looking)"
    python3 -I "$HERE/imx8mp-glmark2-judge.py" "$RECDIR" "$REFDIR" --out "$RECDIR" || step_fail judge "imx8mp-glmark2-judge.py failed"
else
    echo "session: no judge tool; inputs for note-fill: scene '$INFO', commit $sha8, record $relrec, route dump"
fi
echo "session: look at $RECDIR/$LABEL.sidebyside.png, then (not run here) one of:"
for v in right wrong; do
    echo "  python3 tools/cts/imx8mp-glmark2-note-fill.py --note $NOTE --scene $LABEL --commit <full sha of $sha8> --record $relrec --route dump --verdict $v"
done
exit 0
