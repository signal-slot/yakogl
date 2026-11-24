#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi_5/rpi5_hello/verdicts.sh <console.log>
#
# Reads the console log of the rpi5_hello image and checks the count line
# src/main.c prints at the end:
#
#   rpi5_hello: counted 3 s in <ms> ms
#
# PASS when the line says 3 s and <ms> lies in 3000..4000 (three one-second
# sleeps cannot take less than 3000 ms; 1000 ms of slack is far more than a
# healthy timer needs). A later line wins (a log with two boots).
#
# Prints one line:  verdicts: PASS (...) | FAIL (...) | no end line (...)
# Exit status: 0 PASS, 1 no `counted` line, 2 usage, 3 FAIL.
#
# Self-test: tools/probes/rpi_5/rpi5_hello/verdicts_selftest.sh
set -u

if [ $# -ne 1 ]; then
    echo "usage: ${0##*/} <console.log>" >&2
    exit 2
fi
if [ ! -r "$1" ] || [ -d "$1" ]; then
    echo "${0##*/}: cannot read $1" >&2
    exit 2
fi

line=$(tr -d '\r' <"$1" | grep 'rpi5_hello: counted ' | tail -n 1)
if [ -z "$line" ]; then
    echo "verdicts: no end line (no 'rpi5_hello: counted' line in the log)"
    exit 1
fi

set -- $(printf '%s\n' "$line" | sed -n 's/.*rpi5_hello: counted \([0-9][0-9]*\) s in \([0-9][0-9]*\) ms.*/\1 \2/p')
if [ $# -ne 2 ]; then
    echo "verdicts: FAIL (count line not parsable: $line)"
    exit 3
fi
if [ "$1" -ne 3 ]; then
    echo "verdicts: FAIL (counted $1 s, expected 3 s)"
    exit 3
fi
if [ "$2" -lt 3000 ] || [ "$2" -gt 4000 ]; then
    echo "verdicts: FAIL (3 s took $2 ms, expected 3000..4000 ms)"
    exit 3
fi
echo "verdicts: PASS (counted 3 s in $2 ms)"
exit 0
