#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Stage 2 of dEQP on Zephyr, Raspberry Pi 4 (V3D): one image per
# "# image <LABEL>:" block of a table. The builder is
# tools/cts/deqp-tree-images.sh (its header says the arguments and the
# environment); this name gives it the board rpi_4b.
#
#   tools/cts/pi4-deqp-images.sh [--dry-run] <table> <img-dir> [LABEL...]
exec "$(dirname "$0")/deqp-tree-images.sh" rpi_4b "$@"
