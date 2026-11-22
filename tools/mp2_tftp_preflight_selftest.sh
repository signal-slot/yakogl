#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of mp2_tftp_preflight.sh: both verdicts via environment overrides.
set -u
here=$(cd "$(dirname "$0")" && pwd)
p="$here/mp2_tftp_preflight.sh"
fail=0
check() { # name want_rc want_pattern env...
  local name=$1 rc=$2 pat=$3
  shift 3
  out=$(env "$@" "$p" 2>&1)
  got=$?
  v=$(printf '%s\n' "$out" | grep -c '^mp2_tftp_preflight: \(ready\|not ready\)')
  if [ "$got" = "$rc" ] && [ "$v" = 1 ] && printf '%s\n' "$out" | tail -1 | grep -q "$pat"; then
    echo "ok: $name"
  else
    echo "FAIL: $name (rc=$got)"
    echo "$out"
    fail=1
  fi
}
check ready 0 '^mp2_tftp_preflight: ready$' \
  MP2_PREFLIGHT_SYSCTL=0 MP2_PREFLIGHT_BIND=ok MP2_PREFLIGHT_SERVE=ok
check sysctl-high 1 'not ready: .*ip_unprivileged_port_start=1024' \
  MP2_PREFLIGHT_SYSCTL=1024 MP2_PREFLIGHT_BIND=ok MP2_PREFLIGHT_SERVE=ok
check bind-fail 1 'not ready: .*bind of UDP 69 fails' \
  MP2_PREFLIGHT_SYSCTL=0 MP2_PREFLIGHT_BIND=fail
check serve-fail 1 'not ready: .*could not serve' \
  MP2_PREFLIGHT_SYSCTL=0 MP2_PREFLIGHT_BIND=ok MP2_PREFLIGHT_SERVE=fail
[ $fail = 0 ] && echo "mp2_tftp_preflight_selftest: pass"
exit $fail
