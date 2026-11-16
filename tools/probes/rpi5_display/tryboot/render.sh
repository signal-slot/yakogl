#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi5_display/tryboot/render.sh <fw|bare> <image name>
#
# Prints the tryboot.txt for the rpi5_display session to stdout. Writes
# nothing outside a private scratch directory that is removed on exit.
# fw = routes A and B, bare = route C. Exit: 0 ok, 2 bad argument,
# 4 route block missing, 5 placeholder missing.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd) || exit 1
tpl=${TRYBOOT_TEMPLATE:-$here/tryboot.txt.in}
[ $# -eq 2 ] || { echo "usage: render.sh <fw|bare> <image name>" >&2; exit 2; }
variant=$1
name=$2
case $variant in
fw | bare) ;;
*) echo "render.sh: variant must be fw or bare" >&2; exit 2 ;;
esac
[[ $name =~ ^[A-Za-z0-9][A-Za-z0-9._+-]{0,59}$ ]] ||
	{ echo "render.sh: bad image name" >&2; exit 2; }
[ -r "$tpl" ] || { echo "render.sh: template unreadable" >&2; exit 1; }
scratch=$(mktemp -d "${TMPDIR:-/tmp}/tryboot.XXXXXX") || exit 1
trap 'rm -rf "$scratch"' EXIT
route=$scratch/route
out=$scratch/out
awk -v v="$variant" '
	$0 == "# @ROUTE " v { on = 1; next }
	$0 == "# @ENDROUTE" { on = 0; next }
	on { print }
' "$tpl" >"$route"
grep -q '^[a-z_0-9]*=' "$route" || { echo "render.sh: route block $variant missing" >&2; exit 4; }
grep -qx '@ROUTE@' "$tpl" && grep -qx 'kernel=@IMAGE@' "$tpl" ||
	{ echo "render.sh: placeholder missing" >&2; exit 5; }
awk -v n="$name" -v rf="$route" '
	/^# @ROUTE / { skip = 1; next }
	$0 == "# @ENDROUTE" { skip = 0; next }
	skip { next }
	$0 == "@ROUTE@" { while ((getline l < rf) > 0) print l; next }
	$0 == "kernel=@IMAGE@" { print "kernel=" n; next }
	{ print }
' "$tpl" >"$out" || exit 1
# Command substitution drops the trailing blank lines of the removed blocks.
text=$(cat "$out")
printf '%s\n' "$text"
