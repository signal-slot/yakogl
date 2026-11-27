#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6-redact-check.sh - refuse a NanoPC-T6 U-Boot record that still carries
# board identity (plan #408, item t6-redact-check).
#
#   tools/cts/t6-redact-check.sh DIR...        (a single FILE works too)
#
# Every regular file under each DIR (any depth, hidden files included;
# symbolic links and non-regular files are skipped) is scanned for what a
# record under tools/cts/log/<date>-t6-uboot-<sha8>/ may not hold: no
# addresses, no UART paths, no MACs, no USB serial numbers, no UUIDs, no
# host names (CLAUDE.md "Do not touch": environment only, never in a file
# or a commit). Binary files are scanned as byte streams too. Every finding
# is one line
#
#   REDACT <path>:<line>: <kind>
#
# (one per kind and line; the matched text is never echoed, so the report
# does not repeat what it found). At least as strict as plan #408's
# acceptance grep c3 (grep -Ei over every file under the log directories):
# each alternative c3 flags is flagged here, with a kind.
#
# Kinds:
#   rfc2217        the serial bridge's scheme: its name glued to :// (c3)
#   uart           the USB serial converter's tty (c3's device path is a
#                  /dev form of this), anywhere
#   serial-path    a /dev/ser/ path: the by-id names carry the converter's
#                  USB serial number
#   mac            six hex pairs joined by ':' (c3's own, no boundaries) or
#                  by '-' (U-Boot prints the ethaddr both ways)
#   tailnet        an address in 100.64/10 or any partly redacted spelling
#                  of its start: 100 + '.' + 64..127 + '.' flags whatever
#                  follows it, as c3 does (a partly redacted address is
#                  still an address), and a tailnet DNS name
#   ipv4           a dotted quad of 1-3 digit octets 0-255; not reported:
#                  127.x.x.x, 0.x.x.x, and a quad glued to a word, '-' or
#                  '.' on either side (version strings: mesa-24.0.8.1,
#                  v1.2.3.4, 1.2.3.4.5, BVNC 36.53.104.796)
#   ipv6           a '::' address with hex before the '::', or all eight
#                  groups; bare '::' and '::1' are loopback-like and pass
#   uuid           8-4-4-4-12 hex digits (the GPT disk UUIDs U-Boot's
#                  distro boot and its EFI tools print). Not reported: a
#                  UUID on a line naming a boot id (boot_id=, "Boot id:",
#                  as the runner's meta.txt, kmsg-session.txt and README
#                  spell it) — the kernel boot id is random per boot, the
#                  runner's no-reboot evidence, kept by mali-slice-redact
#                  too; every other UUID is a finding
#   usb-serial     the value of a label: iSerial, SerialNumber or Serial
#                  Number, with a token of 4+ characters after it (no
#                  value, or a shorter one, passes)
#   uboot-id       U-Boot's board serial variable (its name, '#', then = or :)
#                  with a value after it;
#                  the placeholder <serial> passes (console-redact.py)
#   userhost       a login name at a host (the shell prompt form, a mail
#                  address, a systemd unit instance), except when the host
#                  part is all hex: those are U-Boot device unit names
#                  (mmc, gpu and watchdog nodes at their hex addresses)
#   extra          a non-empty line of the file named by REDACT_EXTRA,
#                  searched as a fixed string (case-insensitive); for
#                  values the run knows by no other means
#   env:NAME       the value (3+ characters) of BOARD_HOST, BOARD_USER,
#                  BOARD_UART, T6_UART or T6_HOST, searched as a fixed
#                  string: board addresses and names live in the
#                  environment only, so a record holding one is refused by
#                  the caller's own environment
#   unreadable     a directory whose listing failed (find error) or a file
#                  that could not be read: not scanned means not clean
#
# Fail closed: a find failure and unreadable entries count as findings of
# their path (kind unreadable), so an incomplete run never reports clean.
#
# Exit: 0 every file scanned and clean; 1 at least one finding (the
# unreadable kind included); 2 usage, an argument that does not exist or
# is not a real file or directory, or a REDACT_EXTRA naming no readable
# file.
#
# Neither this file nor its selftest trips these rules: every sample they
# hold is built at run time from pieces, the way tools/cts/
# console-redact-selftest.sh does it, and the selftest checks it.
set -u
export LC_ALL=C

usage() {
  sed -n '4,/^set -u$/p' "$0" | sed -e '/^set -u$/d' -e 's/^# \{0,1\}//'
}

if [ $# -eq 0 ]; then
  echo "t6-redact-check: usage: t6-redact-check.sh DIR..." >&2
  exit 2
fi
case "${1-}" in
  -h | --help) usage; exit 0 ;;
esac

T=$(mktemp -d "${TMPDIR:-/tmp}/t6-redact-check.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT HUP INT TERM

# kind, pattern and an optional per-line exclusion, all searched
# case-insensitively as extended regexes. The bridge scheme and the
# device-path rule are written by concatenation, so this file's own text
# passes its own check (and the acceptance grep).
KINDS=()
PATS=()
EXCS=()
addk() { KINDS+=("$1"); PATS+=("$2"); EXCS+=("${3-}"); }
addk rfc2217 'rfc2217'"://"
addk uart 'tty(USB|ACM)'
addk serial-path '/dev/ser''ial/'
addk mac '([0-9a-f]{2}:){5}[0-9a-f]{2}'
addk mac-dash '(^|[^0-9a-f:-])(([0-9a-f]{2}-){5}[0-9a-f]{2})([^0-9a-f-]|$)'
addk tailnet '100\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.'
addk tailnet-name '(^|[^0-9a-z._-])([0-9a-z-]+\.)+ts[.]net([^0-9a-z-]|$)'
addk ipv4 '(^|[^0-9a-z._-])(25[0-5]|2[0-4][0-9]|1(0[0-9]|1[0-9]|2[0-6]|2[89]|[3-9][0-9])|[1-9][0-9]|[1-9])\.(25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])\.(25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])\.(25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])([^0-9a-z._-]|$)'
addk ipv6-colons '(^|[^0-9a-z_.:])[0-9a-f]{1,4}(:[0-9a-f]{1,4})*::'
addk ipv6-full '(^|[^0-9a-z_.:])[0-9a-f]{1,4}(:[0-9a-f]{1,4}){7}([^0-9a-f_.:]|$)'
addk uuid '(^|[^0-9a-f-])[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}([^0-9a-f-]|$)' \
  '(^|[^[:alnum:]])boot[ _-]?id'
addk usb-serial '(iserial|serialnumber|serial number)[[:space:]:=]+([0-9]+[[:space:]]+)?[0-9a-z._-]{4,}'
addk uboot-id 'serial#[[:space:]]*[:=][[:space:]]*[^[:space:]<]'
addk userhost '(^|[^0-9a-z._%+-])[0-9a-z._+-]+@[0-9a-z-]*[g-z][0-9a-z.-]*'

hits=0
missing=0

report() { # report <path> <line> <kind>
  hits=$((hits + 1))
  printf 'REDACT %s:%s: %s\n' "$1" "$2" "$3"
}

scan_grep() { # scan_grep <path> <kind> <-E|-F> <-e|-f> <pattern> [<exclude-ERE>]
  # runs grep to a scratch file (a binary's NUL bytes never pass through a
  # command substitution), searched case-insensitively; reports every
  # matched line number with <kind>, and an unreadable path when grep
  # could not run at all. Lines matching <exclude-ERE> are dropped again.
  local f=$1 kind=$2 rc ln lns excl
  shift 2
  grep "$1ian" "$2" "$3" -- "$f" > "$T/g.out" 2> /dev/null
  rc=$?
  excl=${4-}
  if [ "$rc" -ge 2 ]; then
    report "$f" 0 unreadable
    return
  fi
  [ "$rc" = 1 ] && return
  if [ -n "$excl" ]; then
    grep -Eiv -e "$excl" "$T/g.out" > "$T/g.flt" 2>/dev/null
    mapfile -t lns < <(cut -d: -f1 "$T/g.flt")
  else
    mapfile -t lns < <(cut -d: -f1 "$T/g.out")
  fi
  for ln in ${lns[@]+"${lns[@]}"}; do
    case "$ln" in ('' | *[!0-9]*) continue ;; esac
    report "$f" "$ln" "$kind"
  done
}

scan_file() { # scan_file <path>
  local f=$1 i kind
  if [ ! -r "$f" ]; then report "$f" 0 unreadable; return; fi
  i=0
  while [ "$i" -lt "${#KINDS[@]}" ]; do
    kind=${KINDS[i]}
    scan_grep "$f" "$kind" -E -e "${PATS[i]}" "${EXCS[i]}"
    i=$((i + 1))
  done
  i=0
  while [ "$i" -lt "${#FIXED[@]}" ]; do
    scan_grep "$f" "${FIXED[i]}" -F -e "${FIXED[i + 1]}"
    i=$((i + 2))
  done
  if [ -s "$T/extra.pats" ]; then
    scan_grep "$f" extra -F -f "$T/extra.pats"
  fi
}

scan_dir() { # scan_dir <dir>
  local d=$1 f
  if ! find "$d" -type f -print0 > "$T/list" 2> "$T/find.err"; then
    # fail closed: an unlistable directory is a finding, never a pass.
    report "$d" 0 unreadable
    echo "t6-redact-check: $d: the listing failed: $(head -n 1 "$T/find.err" 2>/dev/null)" >&2
  fi
  while IFS= read -r -d '' f; do scan_file "$f"; done < "$T/list"
}

: > "$T/extra.pats"
FIXED=()
for name in BOARD_HOST BOARD_USER BOARD_UART T6_UART T6_HOST; do
  v=${!name-}
  v=${v//$'\n'/}
  [ "${#v}" -ge 3 ] && FIXED+=("env:$name" "$v")
done
if [ -n "${REDACT_EXTRA-}" ]; then
  if [ ! -f "$REDACT_EXTRA" ] || [ ! -r "$REDACT_EXTRA" ]; then
    echo "t6-redact-check: REDACT_EXTRA: no readable file: $REDACT_EXTRA" >&2
    exit 2
  fi
  while IFS= read -r p || [ -n "$p" ]; do
    p=${p%$'\r'}
    [ -n "$p" ] && printf '%s\n' "$p"
  done < "$REDACT_EXTRA" > "$T/extra.pats"
fi

for p in "$@"; do
  if [ -L "$p" ]; then
    echo "t6-redact-check: refusing a symbolic link: $p" >&2
    missing=1
  elif [ ! -e "$p" ]; then
    echo "t6-redact-check: no such file or directory: $p" >&2
    missing=1
  elif [ -d "$p" ]; then
    scan_dir "$p"
  elif [ -f "$p" ]; then
    scan_file "$p"
  else
    echo "t6-redact-check: not a file or directory: $p" >&2
    missing=1
  fi
done

if [ "$missing" = 1 ]; then exit 2; fi
if [ "$hits" -gt 0 ]; then exit 1; fi
exit 0
