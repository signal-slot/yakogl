#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# imx95_bbm_record.sh - the Verdin iMX95's result record, read from Linux.
#
#   imx95_bbm_record.sh read      the record: "w0 w1 w2 w3", 8 hex digits each
#   imx95_bbm_record.sh clear     GPR4 (w0) zeroed first, then GPR5..7; then read
#   imx95_bbm_record.sh probe     the raw interface's directory, or why there is none
#   imx95_bbm_record.sh encode get GPR | encode set GPR VALUE
#                                 the command's bytes, as od -t x4 prints them
#   imx95_bbm_record.sh decode get|set GPR WORDS...
#                                 a reply's words (header, status, value): the
#                                 value of a get, nothing for a set
#
# Runs on the board as root (tools/board.sh imx95 pipes it through
# "sudo -n sh -s -- <verb>"); POSIX sh, printf, od and cat only. The image
# keeps its last record in the battery-backed BBNSM GPR4..7 (w0 in GPR4),
# written over SCMI to the System Manager's BBM protocol
# (driver/zephyr/lib/board/verdin_imx95/verdin_imx95_record.c,
# driver/rust/src/soc/imx95/bbm.rs; docs/standalone-boot.md, "The record
# without a console"). Linux has no interface to those GPRs: its BBM driver
# (drivers/firmware/arm_scmi/vendors/imx/imx-sm-bbm.c) knows BBM_GPR_GET but
# exports only the RTC and the button. So this script sends the same
# BBM_GPR_GET / BBM_GPR_SET messages itself, through the kernel's SCMI raw
# interface (Documentation/ABI/testing/debugfs-scmi-raw:
# /sys/kernel/debug/scmi/<n>/raw/message: write one command, header
# included, little-endian; read its reply, header, status, values). That
# interface exists only in a kernel built with CONFIG_ARM_SCMI_RAW_MODE_SUPPORT
# and, so that the normal SCMI drivers keep running beside it,
# CONFIG_ARM_SCMI_RAW_MODE_SUPPORT_COEX. Without it "probe" says so and every
# verb exits 3.
#
# The messages: header = message id | 0x81 << 10 (NXP's BBM protocol) |
# token << 18, token 0x10 + GPR (the raw interface takes the caller's
# token); BBM_GPR_SET (0x3) carries GPR, value; BBM_GPR_GET (0x4) carries
# GPR and is answered with header, status 0, value. soc::imx95::bbm's test
# pins the same header words.
#
# Exit codes: 0 done; 2 usage; 3 no SCMI raw interface; 4 the System Manager
# refused (its status printed); 5 no reply, or a reply that does not answer
# the command.
#
# Environment: BBM_RAW the raw directory (default: the first
# /sys/kernel/debug/scmi/*/raw that has a message file); BBM_XFER a command
# that takes the command's bytes on stdin and prints the reply's bytes
# instead of the raw interface (tools/imx95_bbm_record_selftest.sh's fake
# System Manager).
set -u

PROTO=129 # 0x81
GPR_SET=3
GPR_GET=4
FIRST=4 # w0's GPR

die() {
    echo "imx95_bbm_record: $1" >&2
    exit "$2"
}

# le32 VALUE: the four bytes of VALUE, little-endian, as printf escapes
le32() {
    v=$(($1))
    printf '\\%03o\\%03o\\%03o\\%03o' $((v & 255)) $(((v >> 8) & 255)) $(((v >> 16) & 255)) $(((v >> 24) & 255))
}

# header MSG GPR
header() { echo $(($1 | PROTO << 10 | (16 + $2) << 18)); }

# message get GPR | message set GPR VALUE: the command's bytes on stdout
message() {
    case "$1" in
    get) printf "$(le32 "$(header $GPR_GET "$2")")$(le32 "$2")" ;;
    set) printf "$(le32 "$(header $GPR_SET "$2")")$(le32 "$2")$(le32 "$3")" ;;
    *) die "message: get or set, not '$1'" 2 ;;
    esac
}

find_raw() {
    if [ -n "${BBM_RAW:-}" ]; then
        [ -e "$BBM_RAW/message" ] || return 1
        echo "$BBM_RAW"
        return 0
    fi
    for d in /sys/kernel/debug/scmi/*/raw; do
        if [ -e "$d/message" ]; then
            echo "$d"
            return 0
        fi
    done
    return 1
}

need_raw() {
    [ -n "${BBM_XFER:-}" ] && return 0
    RAW=$(find_raw) || die "no SCMI raw interface (no /sys/kernel/debug/scmi/*/raw/message): this kernel lacks CONFIG_ARM_SCMI_RAW_MODE_SUPPORT(_COEX), or debugfs is not mounted; the record cannot be read from Linux" 3
}

# xfer: the command on stdin to the System Manager, its reply's words
# ("header status value...") on stdout
xfer() {
    if [ -n "${BBM_XFER:-}" ]; then
        sh -c "$BBM_XFER" | od -A n -t x4 -v | tr -s ' \n' '  '
        return
    fi
    # one write of the whole command, then the reply (one message, then EOF)
    cat > "$RAW/message" || return 1
    od -A n -t x4 -v < "$RAW/message" | tr -s ' \n' '  '
}

# decode MSG GPR WORDS...: the value of a reply to MSG for GPR, after the
# checks (exit 5: no reply or not this command's; 4: refused)
decode() {
    msg=$1 gpr=$2
    shift 2
    [ $# -ge 2 ] || die "GPR$gpr: no reply" 5
    want=$(printf '%08x' "$(header "$msg" "$gpr")")
    [ "$1" = "$want" ] || die "GPR$gpr: the reply's header is $1, not $want" 5
    st=$(($(printf '0x%s' "$2")))
    if [ "$st" != 0 ]; then
        st=$((st > 2147483647 ? st - 4294967296 : st))
        case "$st" in
        -3) die "GPR$gpr: the System Manager refused: DENIED (the agent has no permission on the GPR)" 4 ;;
        -4) die "GPR$gpr: the System Manager refused: NOT_FOUND" 4 ;;
        *) die "GPR$gpr: the System Manager refused: status $st" 4 ;;
        esac
    fi
    if [ "$msg" = $GPR_GET ]; then
        [ $# -ge 3 ] || die "GPR$gpr: the reply has no value" 5
        echo "$3"
    fi
}

get() {
    # shellcheck disable=SC2046  # the reply's words
    decode $GPR_GET "$1" $(message get "$1" | xfer)
}

set_gpr() {
    # shellcheck disable=SC2046
    decode $GPR_SET "$1" $(message set "$1" "$2" | xfer)
}

read_record() {
    w0=$(get $FIRST) || exit $?
    w1=$(get $((FIRST + 1))) || exit $?
    w2=$(get $((FIRST + 2))) || exit $?
    w3=$(get $((FIRST + 3))) || exit $?
    echo "$w0 $w1 $w2 $w3"
}

usage() {
    echo "usage: imx95_bbm_record.sh read | clear | probe | encode get|set GPR [VALUE] | decode get|set GPR WORDS..." >&2
    exit 2
}
[ $# -ge 1 ] || usage
verb=$1
shift
case "$verb" in
read)
    need_raw
    read_record
    ;;
clear)
    need_raw
    # w0 first: a record half cleared never shows the old magic over zeros
    for g in $FIRST $((FIRST + 1)) $((FIRST + 2)) $((FIRST + 3)); do
        set_gpr "$g" 0 || exit $?
    done
    read_record
    ;;
probe)
    if RAW=$(find_raw); then echo "raw: $RAW"; else
        echo "no SCMI raw interface: CONFIG_ARM_SCMI_RAW_MODE_SUPPORT_COEX not in this kernel, or debugfs not mounted"
        exit 3
    fi
    ;;
encode)
    [ $# -ge 2 ] || usage
    message "$@" | od -A n -t x4 -v | tr -s ' \n' '  ' | sed 's/^ //; s/ $//'
    echo
    ;;
decode)
    [ $# -ge 2 ] || usage
    case "$1" in get) m=$GPR_GET ;; set) m=$GPR_SET ;; *) usage ;; esac
    shift
    decode "$m" "$@"
    ;;
*) usage ;;
esac
