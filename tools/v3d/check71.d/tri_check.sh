#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# tri_check.h, the Pi 5 triangle probe's read-back classifier, on the host.
# Run as `sh tri_check.sh` too: a POSIX shell hands it to bash before its set -o pipefail.
[ -n "${BASH_VERSION:-}" ] || exec bash "$0" "$@"
set -euo pipefail
cd "$(dirname "$0")/../../.."
exec sh tools/probes/rpi_5/common/tri_check/test/run.sh
