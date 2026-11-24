#!/usr/bin/env bash
# Acceptance checks c1-c5 on the i.MX8MP glmark2 pictures-by-eye note, one line each.
#
#   tools/cts/imx8mp-glmark2-accept.sh [--note PATH] [--main REF] [--no-fetch] [--skip c2]
#
# c1 verdicts.py --complete says 'ok: 33 rows, 33 filled'
# c2 verdicts.py --self-test and imx8mp-glmark2-dumps.sh --self-test pass
# c3 every commit in the table's third column is an ancestor of --main
# c4 no skeleton text left, and 33 '- **[' entries
# c5 no address, serial path, tailnet name or password in the note
# Prints 'cN ok' or 'cN FAIL <why>'; exits 0 only if every check passed
# (a skipped c2 prints 'c2 skipped'). Host only: no board, serial, camera or gh.
# Needs bash, python3, git and TMPDIR.
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
NOTE=$ROOT/docs/notes/imx8mp-glmark2-pictures-by-eye.md
MAIN=origin/main
FETCH=1
SKIP2=0
while [ $# -gt 0 ]; do
    case $1 in
    --note) [ $# -ge 2 ] || { echo "--note needs a path" >&2; exit 2; }; NOTE=$2; shift ;;
    --main) [ $# -ge 2 ] || { echo "--main needs a ref" >&2; exit 2; }; MAIN=$2; shift ;;
    --no-fetch) FETCH=0 ;;
    --skip)
        [ "${2:-}" = c2 ] || { echo "--skip: only c2" >&2; exit 2; }
        SKIP2=1; shift ;;
    -h|--help) sed -n 2,13p "$0"; exit 0 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
    esac
    shift
done
[ -f "$NOTE" ] || { echo "no note: $NOTE" >&2; exit 2; }
[ -n "${TMPDIR:-}" ] || { echo "TMPDIR is not set" >&2; exit 2; }
VERD=$HERE/imx8mp-glmark2-verdicts.py
rc=0
res() { # name status why
    if [ "$2" = ok ]; then echo "$1 ok"; else echo "$1 FAIL $3"; rc=1; fi
}

# c1: verdicts.py takes the note path as its first argument.
out=$(python3 "$VERD" "$NOTE" --complete 2>&1); st=$?
if [ $st -eq 0 ] && printf '%s\n' "$out" | grep -qx 'ok: 33 rows, 33 filled'; then
    res c1 ok
else
    res c1 FAIL "exit $st: $(printf '%s\n' "$out" | head -n 1)"
fi

# c2
if [ $SKIP2 -eq 1 ]; then
    echo "c2 skipped"
else
    why=""
    python3 "$VERD" --self-test >/dev/null 2>&1 || why="verdicts.py --self-test failed"
    bash "$HERE/imx8mp-glmark2-dumps.sh" --self-test >/dev/null 2>&1 \
        || why="$why${why:+; }imx8mp-glmark2-dumps.sh --self-test failed"
    if [ -z "$why" ]; then res c2 ok; else res c2 FAIL "$why"; fi
fi

# c3
if [ $FETCH -eq 1 ] && [ "${MAIN#origin/}" != "$MAIN" ]; then
    git -C "$ROOT" fetch -q origin "${MAIN#origin/}" 2>/dev/null \
        || echo "note: git fetch failed, using the local $MAIN" >&2
fi
commits=$(grep -E '^\| \[' "$NOTE" | awk -F'|' '{gsub(/ /,"",$3); print $3}' | sort -u)
why=""
if ! git -C "$ROOT" rev-parse --verify -q "$MAIN^{commit}" >/dev/null; then
    why="no such ref $MAIN"
elif [ -z "$commits" ]; then
    why="no table rows"
else
    for c in $commits; do
        if ! [[ $c =~ ^[0-9a-f]{8}$ ]]; then
            why="$why${why:+, }'$c' is not a commit"
        elif ! git -C "$ROOT" merge-base --is-ancestor "$c" "$MAIN" 2>/dev/null; then
            why="$why${why:+, }$c is not an ancestor of $MAIN"
        fi
    done
fi
if [ -z "$why" ]; then res c3 ok; else res c3 FAIL "$why"; fi

# c4
why=""
for s in 'Not yet established' 'To fill in' 'Skeleton: no scene'; do
    if grep -qF "$s" "$NOTE"; then why="$why${why:+; }contains '$s'"; fi
done
n=$(grep -c '^- \*\*\[' "$NOTE")
if [ "$n" != 33 ]; then why="$why${why:+; }$n '- **[' entries, want 33"; fi
if [ -z "$why" ]; then res c4 ok; else res c4 FAIL "$why"; fi

# c5
hits=$(grep -n -E '([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|tail[a-z0-9-]*\.ts\.net|password' "$NOTE" \
    | cut -d: -f1 | head -n 5 | paste -sd, -)
if [ -z "$hits" ]; then res c5 ok; else res c5 FAIL "match on line(s) $hits"; fi

exit $rc
