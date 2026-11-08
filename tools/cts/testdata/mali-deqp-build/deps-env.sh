#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Stand-in for tools/cts/deps-env.sh in mali-deqp-build-selftest.sh.
if [ "${1-}" = --layout ]; then
	echo "${FAKE_DEPS:?}"
	exit 0
fi
echo "deps-env.sh $*" >> "${FAKE_CALLED:?}"
exit 99
