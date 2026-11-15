#!/usr/bin/env bash
# Record directory helper for the Pi 5 display session (U3).
#
#   record.sh new <name>                    create records/rpi5/<YYYYMMDD-HHMM>-<name>-<sha8>/, print its path
#   record.sh throttled <dir> <label> [v]   append "<label> get_throttled=0x..." to <dir>/throttled.txt
#                                           (v, or stdin: "0x50000" or vcgencmd's "throttled=0x50000")
#   record.sh redact <dir>                  rewrite every text file in place, secrets -> placeholders
#   record.sh check <dir>                   exit 1 if anything redactable is left
#
# Redacted: IPv4/IPv6 addresses, MAC addresses, /dev/tty* and /dev/serial/*
# paths, USB serial strings, and the names in REDACT_NAMES (comma-separated,
# case-insensitive). Memory addresses (0x...) are kept: U3 needs the scanout
# address in the record. This script never runs ssh; the session obtains
# get_throttled over ssh and passes it in. RECORDS_ROOT overrides the
# records/rpi5 location (the self-test uses it).
set -euo pipefail

die() { echo "record.sh: $*" >&2; exit 2; }

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# Print the sed -E script that redacts; REDACT_NAMES is appended.
sed_script() {
    local h='[0-9A-Fa-f]'
    cat <<SED
s#/dev/serial/[^[:space:]]+#/dev/serial/<redacted>#g
s#/dev/tty[A-Za-z0-9_.-]+#/dev/tty<redacted>#g
s#(^|[^0-9A-Fa-f:-])(${h}{2}[:-]){5}${h}{2}([^0-9A-Fa-f:-]|\$)#\1<mac>\3#g
s#(${h}{1,4}:){7}${h}{1,4}#<ip6>#g
s#(${h}{1,4}:){1,7}:(${h}{1,4}(:${h}{1,4}){0,6})?#<ip6>#g
s#::(${h}{1,4}:){0,6}${h}{1,4}#<ip6>#g
s#\b([0-9]{1,3}\.){3}[0-9]{1,3}\b#<ip4>#g
s#(iSerial[[:space:]]+[0-9]+[[:space:]]+)[^[:space:]]+#\1<usb-serial>#Ig
s#(serial(_short|[ _]?number)?[[:space:]]*[=:][[:space:]]*)[^[:space:]]+#\1<usb-serial>#Ig
SED
    local IFS=, n e
    for n in ${REDACT_NAMES:-}; do
        [ -n "$n" ] || continue
        e=$(printf '%s' "$n" | sed -e 's/[][\.*^$+?(){}|#\\/]/\\&/g')
        printf 's#%s#<redacted-name>#Ig\n' "$e"
    done
}

text_files() { # NUL-separated list of text files under $1
    local f
    while IFS= read -r -d '' f; do
        if grep -Iq '' "$f" 2>/dev/null; then printf '%s\0' "$f"; fi
    done < <(find "$1" -type f -print0)
}

cmd_new() {
    [ $# -eq 1 ] || die "usage: new <name>"
    [[ $1 =~ ^[A-Za-z0-9_.-]+$ ]] || die "bad name '$1' (letters, digits, _ . -)"
    local root sha dir
    root=${RECORDS_ROOT:-$(git -C "$here" rev-parse --show-toplevel)/records/rpi5}
    sha=$(git -C "$here" rev-parse --short=8 HEAD)
    dir=$root/$(date +%Y%m%d-%H%M)-$1-$sha
    [ ! -e "$dir" ] || die "$dir exists"
    mkdir -p "$dir"
    echo "$dir"
}

cmd_throttled() {
    [ $# -ge 2 ] && [ $# -le 3 ] || die "usage: throttled <dir> <label> [value]"
    local dir=$1 label=$2 v
    [ -d "$dir" ] || die "no such directory: $dir"
    [[ $label =~ ^[A-Za-z0-9_.:-]+$ ]] || die "bad label '$label'"
    if [ $# -eq 3 ]; then v=$3; else IFS= read -r v || true; fi
    v=${v#throttled=}
    v=${v//[[:space:]]/}
    [[ $v =~ ^0[xX][0-9A-Fa-f]+$ ]] || die "not a hex value: '$v'"
    printf '%s get_throttled=%s\n' "$label" "$v" >>"$dir/throttled.txt"
}

cmd_redact() {
    [ $# -eq 1 ] && [ -d "$1" ] || die "usage: redact <dir>"
    local s t f
    s=$(mktemp); t=$(mktemp)
    sed_script >"$s"
    while IFS= read -r -d '' f; do
        sed -E -f "$s" "$f" >"$t"
        cmp -s "$t" "$f" || cat "$t" >"$f"
    done < <(text_files "$1")
    rm -f "$s" "$t"
}

cmd_check() {
    [ $# -eq 1 ] && [ -d "$1" ] || die "usage: check <dir>"
    local s f bad=0
    s=$(mktemp)
    sed_script >"$s"
    while IFS= read -r -d '' f; do
        if ! sed -E -f "$s" "$f" | cmp -s - "$f"; then
            echo "record.sh: unredacted content in $f" >&2
            bad=1
        fi
    done < <(text_files "$1")
    rm -f "$s"
    return "$bad"
}

[ $# -ge 1 ] || die "usage: record.sh new|throttled|redact|check ..."
sub=$1; shift
case $sub in
    new) cmd_new "$@" ;;
    throttled) cmd_throttled "$@" ;;
    redact) cmd_redact "$@" ;;
    check) cmd_check "$@" || exit 1 ;;
    *) die "unknown subcommand '$sub'" ;;
esac
