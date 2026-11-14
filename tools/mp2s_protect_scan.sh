#!/usr/bin/env bash
# mp2s_protect_scan.sh [file...]
#
# Static scan of the STM32MP257F-DK standalone sources (docs/notes/
# stm32mp257f-dk-standalone.md): none may write
#   - the U-Boot variables bootcmd, bootcount, altbootcmd;
#   - the microSD partitions fsbla1 fsbla2 metadata1 metadata2 fip-a fip-b
#     (by label or /dev/disk/by-partlabel/<name>);
#   - anything on the eMMC (mmcblk0*, U-Boot "mmc 0" / "mmc dev 0").
# Reads (fw_printenv, cat, sha256sum, dd if=, ...) pass. It is a line scan:
# a command split over lines or built from variables is not seen.
#
# Default files: driver/tools/mp2-standalone/*.cmd,
# tools/board-stm32mp2-standalone.sh (if present), tools/mp2s_*.py,
# tools/mp2s_*.sh. A line the scan misreads may end in the exact marker
#   # mp2s-protect: read-only
# (whole-line comments are ignored anyway; any other spelling is no marker).
#
# Prints file:line: text for each hit. Exit 0 clean, 1 hit, 2 usage.
set -u

MARKER='# mp2s-protect: read-only'
ME=${0##*/}

usage() { echo "usage: $ME [file...]" >&2; exit 2; }

case "${1:-}" in -h|--help) usage ;; esac

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
files=("$@")
if [ ${#files[@]} -eq 0 ]; then
	shopt -s nullglob
	files=("$root"/driver/tools/mp2-standalone/*.cmd)
	[ -f "$root/tools/board-stm32mp2-standalone.sh" ] && files+=("$root/tools/board-stm32mp2-standalone.sh")
	files+=("$root"/tools/mp2s_*.py "$root"/tools/mp2s_*.sh)
fi
for f in "${files[@]}"; do
	[ -f "$f" ] && [ -r "$f" ] || { echo "$ME: cannot read $f" >&2; exit 2; }
done

W='[[:space:]]'
NAME='(bootcmd|bootcount|altbootcmd)'
PART='(fsbla1|fsbla2|metadata1|metadata2|fip-a|fip-b)'
# a protected partition or the eMMC
PROT="(${PART}([[:space:]\"';|&)]|\$)|mmcblk0|mmc${W}+(dev${W}+)?0([^0-9]|$))"
TOKP="[^[:space:]]*${PROT}"

# env writes: setenv / env set / fw_setenv with a protected name as argument
RE_ENV="(^|[^[:alnum:]_])(setenv|fw_setenv|env${W}+set)(${W}+-[[:alnum:]]+)*${W}+[\"']?${NAME}([^[:alnum:]_]|$)"
# fw_setenv with options before the name
RE_FW="(^|[^[:alnum:]_])fw_setenv${W}.*${W}[\"']?${NAME}([^[:alnum:]_]|$)"
# list forms: "fw_setenv", "bootcmd"
RE_ENVL="fw_setenv[\"'],?${W}*[\"']${NAME}([^[:alnum:]_]|[\"']|$)"
# dd of=, redirects, tee, cp/mv/install to a protected target
RE_DD="(^|[^[:alnum:]_])dd${W}.*of=[\"']?${TOKP}"
RE_REDIR="(^|[^0-9&<])>>?${W}*[\"']?${TOKP}"
RE_TEE="(^|[^[:alnum:]_])tee${W}([^|;&]*${W})?[\"']?${TOKP}"
RE_CP="(^|[^[:alnum:]_])(cp|mv|install)${W}.*${W}[\"']?${TOKP}[\"';]*${W}*\$"
# device tools and U-Boot writers with a protected thing on the line
RE_DEV="(^|[^[:alnum:]_])(mkfs[.[:alnum:]]*|sgdisk|sfdisk|parted|flash_erase|flashcp|wipefs|blkdiscard|mmc${W}+(write|erase)|ext4write|fatwrite|gpt${W}+write)(${W}|\$).*${PROT}"
# selecting the eMMC in U-Boot is refused outright (later writes would hit it)
RE_SEL="(^|[^[:alnum:]_])mmc${W}+dev${W}+0([^0-9]|\$)"

hits=0
for f in "${files[@]}"; do
	n=0
	while IFS= read -r line || [ -n "$line" ]; do
		n=$((n + 1))
		case "$line" in
		*"$MARKER")
			rest=${line%"$MARKER"}
			case "$rest" in "" | *[[:space:]]) continue ;; esac ;;
		esac
		t=${line#"${line%%[![:space:]]*}"}
		case "$t" in "#"* | "") continue ;; esac
		if printf '%s\n' "$line" | grep -Eq -e "$RE_ENV" -e "$RE_ENVL" -e "$RE_FW" -e "$RE_DD" \
			-e "$RE_REDIR" -e "$RE_TEE" -e "$RE_CP" -e "$RE_DEV" -e "$RE_SEL"; then
			printf '%s:%d: %s\n' "$f" "$n" "$line"
			hits=$((hits + 1))
		fi
	done < "$f"
done

if [ "$hits" -gt 0 ]; then
	echo "$ME: $hits protected write form(s)" >&2
	exit 1
fi
exit 0
