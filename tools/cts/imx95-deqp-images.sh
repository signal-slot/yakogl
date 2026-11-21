#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Stage 2 of dEQP on Zephyr, Verdin iMX95 (i.MX 95, Mali-G310): one image per
# "# image <LABEL>:" block of a table (tools/cts/mali-gles2-tree.txt, the
# NanoPC-T6's rows). The builder is tools/cts/deqp-tree-images.sh (its header
# says the arguments, the environment and what the run script provides); this
# name gives it the board verdin_imx95/mimx9596/a55. Stage 1 is
# `tools/cts/deps-env.sh --build DEQP_WORK --board verdin_imx95`.
#
#   tools/cts/imx95-deqp-images.sh [--dry-run] <table> <img-dir> [LABEL...]
exec "$(dirname "$0")/deqp-tree-images.sh" verdin_imx95/mimx9596/a55 "$@"
