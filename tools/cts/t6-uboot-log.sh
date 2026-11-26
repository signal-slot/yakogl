#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6-uboot-log.sh - copy a redacted NanoPC-T6 U-Boot record into tools/cts/log
# (plan #408, item t6-log-tool).
#
#   tools/cts/t6-uboot-log.sh [--exclude NAME]... RECORD-DIR DATE NAME SHA8
#
#   RECORD-DIR  a board record, e.g. records/nanopc-t6/<YYYYMMDD-HHMM>-<name>-<sha8>/
#               (gitignored; it is only read, never changed)
#   DATE        YYYY-MM-DD or YYYY-MM (the log directory's date)
#   NAME        lower-case letters, digits and '-' (e.g. s2, gl-test)
#   SHA8        the image commit, 8 lower-case hex digits
#   --exclude NAME  leave out every file of that base name (a glob, e.g.
#               answers.txt); repeatable
#
# The destination is tools/cts/log/<DATE>-t6-uboot-<NAME>-<SHA8>/ (the
# directory T6_UBOOT_LOG_ROOT names instead of tools/cts/log, for the
# selftest). Every regular text file under RECORD-DIR (any depth: the
# console uart.log and image.log, typed.log, record.txt, answers.txt,
# verdict.txt, the prompt-go and prompt-reads output) is copied to the same
# relative path through tools/cts/console-redact.py (dotted quads, UART
# paths, home paths and MAC addresses are replaced). Binary files (a NUL
# byte), symbolic links and other non-regular entries are left out and
# named on stderr.
#
# The copy is built in a hidden scratch directory beside the destination
# and refused (exit 1, the scratch directory removed, nothing left behind)
# if any file or path name of it still matches
#   - the acceptance grep (grep -Ei): the serial bridge's URL scheme, a USB
#     or ACM tty path, a ':'-joined MAC, a tailnet address start (100.64/10);
#   - a UUID (8-4-4-4-12 hex digits, any case; a boot_id line too);
#   - tools/cts/t6-redact-check.sh (serial numbers, host names, the values
#     of BOARD_HOST, BOARD_USER, BOARD_UART, T6_UART and T6_HOST, ...).
# A refusal names the file and the kind, never the matched text. To land a
# refused record, copy the record, edit the copy by hand (or leave the file
# out with --exclude) and run this tool on the edited copy.
# Only a clean copy is renamed into place. An existing destination (even an
# empty directory) is never overwritten or changed.
#
# Exit: 0 copied; 1 refused (a secret survived the redaction); 2 usage, a
# bad argument, no text file to copy or a failed copy; 3 the destination
# already exists.
set -u
export LC_ALL=C

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
redactor=$here/console-redact.py
checker=$here/t6-redact-check.sh

usage() {
  sed -n '4,/^set -u$/p' "$0" | sed -e '/^set -u$/d' -e 's/^# \{0,1\}//'
}
die() { echo "t6-uboot-log: $*" >&2; exit 2; }

excludes=()
while [ $# -gt 0 ]; do
  case "$1" in
    -h | --help) usage; exit 0 ;;
    --exclude)
      [ $# -ge 2 ] && [ -n "$2" ] || die "--exclude needs a NAME"
      excludes+=("$2"); shift 2 ;;
    --) shift; break ;;
    -*) die "unknown option: $1" ;;
    *) break ;;
  esac
done
if [ $# -ne 4 ]; then
  echo "t6-uboot-log: usage: t6-uboot-log.sh [--exclude NAME]... RECORD-DIR DATE NAME SHA8" >&2
  exit 2
fi
rec=$1 date=$2 name=$3 sha=$4

[ -L "$rec" ] && die "refusing a symbolic link as RECORD-DIR: $rec"
[ -d "$rec" ] || die "no such record directory: $rec"
[[ $date =~ ^20[0-9]{2}-(0[1-9]|1[0-2])(-(0[1-9]|[12][0-9]|3[01]))?$ ]] || die "DATE is not YYYY-MM-DD or YYYY-MM: $date"
[[ $name =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$ ]] || die "NAME is not lower-case letters, digits and '-': $name"
[[ $sha =~ ^[0-9a-f]{8}$ ]] || die "SHA8 is not 8 lower-case hex digits: $sha"
[ -f "$redactor" ] || die "missing $redactor"
[ -f "$checker" ] || die "missing $checker"

root=${T6_UBOOT_LOG_ROOT:-$here/log}
[ -d "$root" ] || die "no log directory: $root"
root=$(cd "$root" && pwd)
dest=$root/$date-t6-uboot-$name-$sha
rec=$(cd "$rec" && pwd)
case "$rec/" in "$dest"/* | "$root"/*) die "RECORD-DIR lies inside the log directory: $rec" ;; esac
if [ -e "$dest" ] || [ -L "$dest" ]; then
  echo "t6-uboot-log: the destination exists, left as it is: $dest" >&2
  exit 3
fi

stage=$(mktemp -d "$root/.t6-uboot-log.XXXXXX") || die "cannot make a scratch directory in $root"
trap 'rm -rf "$stage"' EXIT HUP INT TERM
fail_copy() { echo "t6-uboot-log: $*" >&2; exit 2; }

# the file list, NUL-separated; find fails closed
find "$rec" -mindepth 1 -print0 > "$stage/.list" 2> "$stage/.find.err" ||
  fail_copy "the record listing failed: $(head -n 1 "$stage/.find.err")"
mkdir "$stage/out" || fail_copy "mkdir failed"
ncopied=0 nskipped=0
while IFS= read -r -d '' src; do
  rel=${src#"$rec"/}
  base=${rel##*/}
  if [ -L "$src" ]; then
    echo "t6-uboot-log: skipped (symbolic link): $rel" >&2; nskipped=$((nskipped + 1)); continue
  fi
  [ -d "$src" ] && continue
  if [ ! -f "$src" ]; then
    echo "t6-uboot-log: skipped (not a regular file): $rel" >&2; nskipped=$((nskipped + 1)); continue
  fi
  skip=0
  for x in ${excludes[@]+"${excludes[@]}"}; do
    # shellcheck disable=SC2053  # the exclude NAME is a glob on purpose
    [[ $base == $x ]] && skip=1
  done
  if [ "$skip" = 1 ]; then
    echo "t6-uboot-log: skipped (--exclude): $rel" >&2; nskipped=$((nskipped + 1)); continue
  fi
  [ -r "$src" ] || fail_copy "cannot read: $rel"
  if ! tr -d '\000' < "$src" | cmp -s - "$src"; then
    echo "t6-uboot-log: skipped (binary): $rel" >&2; nskipped=$((nskipped + 1)); continue
  fi
  mkdir -p "$stage/out/$(dirname "$rel")" || fail_copy "mkdir failed for $rel"
  python3 -I "$redactor" < "$src" > "$stage/out/$rel" || fail_copy "the redactor failed on $rel"
  ncopied=$((ncopied + 1))
done < "$stage/.list"
[ "$ncopied" -gt 0 ] || fail_copy "no text file to copy in $rec"

# refusals: the acceptance grep and UUIDs over contents and path names,
# then t6-redact-check.sh; the matched text is never printed
acc='rfc2217'"://"'|/dev/''tty(USB|ACM)|([0-9a-f]{2}:){5}[0-9a-f]{2}|100\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.'
uuid='[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'
refused=0
refuse() { echo "t6-uboot-log: REFUSED $1: $2" >&2; refused=1; }
for kind in acc uuid; do
  pat=${!kind}
  label=$([ "$kind" = acc ] && echo "acceptance grep" || echo "uuid")
  while IFS= read -r f; do
    refuse "${f#"$stage/out/"}" "$label"
  done < <(grep -rEil -- "$pat" "$stage/out")
  (cd "$stage/out" && find . -mindepth 1) > "$stage/.names"
  if grep -Eiq -- "$pat" "$stage/.names"; then refuse "(a path name)" "$label"; fi
done
chk=$(bash "$checker" "$stage/out" 2>&1)
case $? in
  0) ;;
  1) while IFS= read -r l; do
       case "$l" in "REDACT $stage/out/"*) l=${l#"REDACT $stage/out/"}; refuse "${l%%: *}" "t6-redact-check ${l##*: }" ;;
         *) refuse "(checker)" "$l" ;; esac
     done <<< "$chk" ;;
  *) refuse "(checker)" "t6-redact-check.sh could not run: $(head -n 1 <<< "$chk")" ;;
esac
if [ "$refused" = 1 ]; then
  echo "t6-uboot-log: refused, nothing written to $root" >&2
  exit 1
fi

# into place: never over an existing entry
if [ -e "$dest" ] || [ -L "$dest" ]; then
  echo "t6-uboot-log: the destination appeared meanwhile, left as it is: $dest" >&2
  exit 3
fi
mv -n -T "$stage/out" "$dest" 2>/dev/null
if [ -d "$stage/out" ]; then
  echo "t6-uboot-log: the destination appeared meanwhile, left as it is: $dest" >&2
  exit 3
fi
echo "t6-uboot-log: $ncopied file(s) copied, $nskipped skipped, into $dest"
exit 0
