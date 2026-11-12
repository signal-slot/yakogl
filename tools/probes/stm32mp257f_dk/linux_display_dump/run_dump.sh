#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host wrapper: run linux_display_dump.py on the DK over ssh and print the
# dump on stdout. The board is named by the environment only:
#   DK_SSH        the ssh destination (user@host or an ssh config name)
#   DK_SSH_OPTS   optional extra ssh options (word-split)
# The script and the read list travel over the connection; nothing is
# written on the board. Usage:
#   DK_SSH=... tools/probes/stm32mp257f_dk/linux_display_dump/run_dump.sh \
#       > driver/docs/stm32mp257f-dk-display-dump-linux.txt
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
allow=${ALLOW_FILE:-$repo/driver/docs/stm32mp257f-dk-display-allow.txt}
: "${DK_SSH:?set DK_SSH to the board's ssh destination}"
b64=$(base64 < "$here/linux_display_dump.py" | tr -d '\n')
# shellcheck disable=SC2086
ssh -o BatchMode=yes ${DK_SSH_OPTS:-} "$DK_SSH" \
    "python3 -c 'import sys,base64;sys.argv[0]=\"linux_display_dump.py\";exec(base64.b64decode(sys.argv.pop(1)))' $b64 --readlist -" \
    < "$allow"
