#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Redaction check for the STM32MP257F-DK console logs committed under
# tools/cts/log/*-mp2-gltest-*/ and tools/cts/log/*-mp2-gles2-*/ (plan #405).
#
# usage: tools/cts/mp2-log-redact-check.sh [FILE|DIR...]
#   A directory is scanned recursively (text files only). With no argument the
#   logs under tools/cts/log/*mp2* are scanned.
#   Exit 0: clean; 1: hits; 2: usage or unreadable input.
#
# Output: `file:line: class: masked`; the match is shown as its first
# character and "***", never in full. The summary counts the files with at
# least one hit out of the files read: `N hit(s) in M of K scanned file(s)`
# (K is larger than the number of shard logs: a run directory holds them,
# their .license sidecars and the cases/manifest/times records), and
# `clean (K file(s))` when N is 0.
#
# Classes: ipv4, tty, tailnet, mac, usb-serial.
# Acceptance c6's pattern is the superset of ipv4/tty/tailnet:
#   ([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|tailnet|\.ts\.net
# Its ipv4 part also matches version strings, so an ipv4 match is dropped
# only when it is a known-benign form (the explicit list):
#   - 0.0.0.0, 127.0.0.0/8 and 255.x.x.x (masks);
#   - 192.0.2.0/24 (RFC 5737 TEST-NET-1), and only that range with a valid
#     last octet: tools/board-stm32mp2.sh redacts each run's console copies
#     by mapping every real IPv4 address to 192.0.2.n (its header, step 7),
#     so a committed DK log holds these placeholders, on megasas, icmp and
#     "SSH Per-Connection Server" lines among others. The neighbours stay
#     hits: 192.0.20.1, 192.0.3.1, 192.0.1.2, 192.0.2.256; so do 10.x,
#     100.64.x, 192.168.x and the other RFC 5737 ranges (198.51.100.0/24,
#     203.0.113.0/24) — the selftest's hit fixtures use TEST-NET-2 exactly
#     to keep the distinction pinned;
#   - a quad with a one-digit first octet (2.12.0.1, 4.6.2.14): no address
#     in use starts below 10, so it is a version or section number (the same
#     rule as tools/redact-diff-scan.py);
#   - a quad that is part of a longer dotted number (1.4.6.2.555);
#   - an exact string in the file named by $MP2_REDACT_ALLOW (one per line)
#     or in BENIGN below (empty: nothing else is known benign today).
# The classes and exemptions follow tools/redact-diff-scan.py (ipv4, tty,
# mac, tailnet); it is not imported, because it scans git ranges and this
# scans files. The usb-serial class is new here.
# tools/cts/mp2_log_redact_check_selftest.sh checks it.
set -u

# shellcheck disable=SC2034  # documents the acceptance pattern; selftest greps it
C6='([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|tailnet|\.ts\.net'  # redact-scan: ok
BENIGN=""

die() { echo "mp2-log-redact-check: $*" >&2; exit 2; }

benign_ip() { # $1 = quad
  local ip=$1 o1
  case $ip in
    0.0.0.0|127.*|255.*) return 0 ;;
  esac
  # The DK runner writes these: tools/board-stm32mp2.sh maps each real
  # address of a run's console copies to 192.0.2.n (RFC 5737 TEST-NET-1).
  # Exactly that /24 with a valid last octet, nothing else: the arm lets
  # 192.0.2.256 fall through, and 192.0.20.1, 192.0.3.1 and 192.0.1.2 never
  # enter it.
  case $ip in 192.0.2.*) [ "$((10#${ip##*.}))" -le 255 ] && return 0 ;; esac
  o1=${ip%%.*}
  [ "${#o1}" -eq 1 ] && return 0
  case " $BENIGN " in *" $ip "*) return 0 ;; esac
  if [ -n "${MP2_REDACT_ALLOW:-}" ] && [ -f "$MP2_REDACT_ALLOW" ]; then
    grep -qxF -- "$ip" "$MP2_REDACT_ALLOW" && return 0
  fi
  return 1
}

hits=0
hitfiles=0
lastfile=
report() { # file line class match
  local m=$4
  [ "$1" = "$lastfile" ] || { lastfile=$1; hitfiles=$((hitfiles + 1)); }
  printf '%s:%s: %s: %s***\n' "$1" "$2" "$3" "${m:0:1}"
  hits=$((hits + 1))
}

# each grep prints line:match
scan_file() {
  local f=$1 ln m
  while IFS=: read -r ln m; do
    [ -n "$ln" ] || continue
    benign_ip "$m" || report "$f" "$ln" ipv4 "$m"
  done < <(grep -noP '(?<![\w.])(?:\d{1,3}\.){3}\d{1,3}(?!\w|\.\d)' -- "$f" 2>/dev/null)
  while IFS=: read -r ln m; do
    [ -n "$ln" ] && report "$f" "$ln" tty "$m"
  done < <(grep -noE '/dev/tty[A-Za-z]*[0-9]*|ttyUSB[0-9]*|ttyACM[0-9]*|/dev/serial/[^[:space:]]*' -- "$f" 2>/dev/null)
  while IFS=: read -r ln m; do
    [ -n "$ln" ] && report "$f" "$ln" tailnet "$m"
  done < <(grep -noiE '[A-Za-z0-9.-]*\.ts\.net|tailnet' -- "$f" 2>/dev/null)
  while IFS=: read -r ln m; do
    [ -n "$ln" ] || continue
    case ${m,,} in
      00:00:00:00:00:00|00-00-00-00-00-00|ff:ff:ff:ff:ff:ff|ff-ff-ff-ff-ff-ff) continue ;;
    esac
    report "$f" "$ln" mac "$m"
  done < <(grep -noP '(?<![0-9A-Za-z:-])[0-9A-Fa-f]{2}([:-])(?:[0-9A-Fa-f]{2}\1){4}[0-9A-Fa-f]{2}(?![0-9A-Za-z:-])' -- "$f" 2>/dev/null)
  while IFS=: read -r ln m; do
    [ -n "$ln" ] && report "$f" "$ln" usb-serial "$m"
  done < <(grep -noiE 'i?serial[ _-]?(number)?[ :=]+[A-Za-z]*[0-9][0-9A-Za-z]{5,}|usb-[A-Za-z0-9_.]+-[A-Za-z0-9_.]+-if[0-9]{2}' -- "$f" 2>/dev/null)
}

if [ $# -eq 0 ]; then
  set -- "$(cd "$(dirname "$0")" && pwd)"/log/*mp2*
fi
n=0
for p in "$@"; do
  if [ -d "$p" ]; then
    while IFS= read -r -d '' f; do
      scan_file "$f"; n=$((n + 1))
    done < <(grep -rIlZ '' -- "$p" 2>/dev/null)
  elif [ -f "$p" ]; then
    scan_file "$p"; n=$((n + 1))
  elif [ "${p##*/}" = '*mp2*' ]; then
    : # the default glob matched nothing
  else
    die "not a file or directory: $p"
  fi
done
if [ "$hits" -gt 0 ]; then
  echo "mp2-log-redact-check: $hits hit(s) in $hitfiles of $n scanned file(s)" >&2
  exit 1
fi
echo "mp2-log-redact-check: clean ($n file(s))"
