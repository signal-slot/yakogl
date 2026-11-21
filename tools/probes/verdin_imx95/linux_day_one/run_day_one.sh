#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host wrapper: run day_one.sh on the Verdin iMX95's Linux over ssh, with
# the M4 read list (driver/docs/imx95-display-allow.txt), and write the
# record to stdout. The board is named by the environment only:
#   VI95_SSH       the ssh destination (user@host or an ssh config name)
#   VI95_SSH_OPTS  optional extra ssh options (word-split)
#   VI95_SUDO      optional prefix when the login is not root ("sudo -n")
# The script and the read list travel over the connection; nothing is
# written on the board. Usage:
#   VI95_SSH=... tools/probes/verdin_imx95/linux_day_one/run_day_one.sh [day_one.sh options] > record.txt
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
allow=${ALLOW_FILE:-$repo/driver/docs/imx95-display-allow.txt}
: "${VI95_SSH:?set VI95_SSH to the board's ssh destination}"
b64=$(base64 < "$here/day_one.sh" | tr -d '\n')
# shellcheck disable=SC2086
ssh -o BatchMode=yes ${VI95_SSH_OPTS:-} "$VI95_SSH" \
	"${VI95_SUDO:-} sh -c \"\$(echo $b64 | base64 -d)\" day_one.sh --readlist - $*" < "$allow"
