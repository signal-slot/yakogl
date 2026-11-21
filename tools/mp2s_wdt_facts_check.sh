#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Check the OP-TEE watchdog SMC facts in driver/rust/src/soc/stm32mp2/smc_wdt*.rs
# and tools/probes/stm32mp257f_dk/mp2_wdt/src/mp2_wdtr.h against the fixture
# tools/testdata/mp2s-wdt/facts.txt (each line cites upstream path:line), and,
# when ST_OPTEE_R31_TREE / ST_TFA_R31_TREE are set, against those trees.
# Exit 0 all agree; 1 with "MISMATCH: ..." lines; 2 usage.
# Env: MP2S_WDT_ROOT (repo root, default: this script's), MP2S_WDT_FACTS (fixture).
set -u
case "${1:-}" in -h|--help) echo "usage: $0 (no arguments; env ST_OPTEE_R31_TREE, ST_TFA_R31_TREE optional)"; exit 0;; esac
[ $# -eq 0 ] || { echo "usage: $0 (no arguments)" >&2; exit 2; }
root=${MP2S_WDT_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
facts=${MP2S_WDT_FACTS:-$root/tools/testdata/mp2s-wdt/facts.txt}
rs=$root/driver/rust/src/soc/stm32mp2
hdr=$root/tools/probes/stm32mp257f_dk/mp2_wdt/src/mp2_wdtr.h
for f in "$facts" "$rs/smc_wdt.rs" "$rs/smc_wdt_refresh.rs" "$hdr"; do
	[ -r "$f" ] || { echo "usage: cannot read $f" >&2; exit 2; }
done
bad=0
mm() { echo "MISMATCH: $*"; bad=1; }
norm() {
	local v=$1
	v=${v//_/}
	v=${v%u}
	if [[ $v =~ ^0[xX][0-9a-fA-F]+$ ]]; then printf '0x%x' "$((v))"
	elif [[ $v =~ ^[0-9]+$ ]]; then printf '%d' "$((10#$v))"
	else printf '%s' "$v"; fi
}
# cap REGEX FILE: capture group 1 of the first line matching REGEX (ERE)
cap() { sed -nE "s/$1/\\1/p" "$2" | head -n1; }
declare -A want
while read -r key val src; do
	case "$key" in ''|'#'*) continue;; esac
	[ -n "${src:-}" ] || { echo "usage: bad fixture line: $key $val" >&2; exit 2; }
	nv=$(norm "$val")
	[ -z "${want[$key]:-}" ] || [ "${want[$key]}" = "$nv" ] || mm "fixture: $key has two values (${want[$key]}, $nv)"
	want[$key]=$nv
	case "$src" in derived:*) ;; *:*:[0-9]*) ;; *) mm "fixture $key has no upstream path:line ($src)";; esac
done < <(grep -v '^[[:space:]]*#' "$facts")
for k in SMC_FUNC_ID CMD_INIT CMD_SET_TIMEOUT CMD_ENABLE CMD_PET CMD_GET_TIMELEFT TIMEOUT_S EARLY_S MIN_TIMEOUT_S; do
	[ -n "${want[$k]:-}" ] || mm "fixture lacks $k"
done

cmp1() { # label key got
	if [ -z "$3" ]; then mm "$1: value not found for $2"; return; fi
	local g
	g=$(norm "$3")
	[ "$g" = "${want[$2]:-}" ] || mm "$1: $2 is $g, fixture has ${want[$2]:-?}"
}
r=$rs/smc_wdt_refresh.rs
cmp1 smc_wdt_refresh.rs SMC_FUNC_ID "$(cap '^pub const FUNC_ID_DK: u32 = ([0-9a-fA-Fx_]+);.*' "$r")"
cmp1 smc_wdt_refresh.rs TIMEOUT_S "$(cap '^pub const TIMEOUT_DK_S: u32 = ([0-9_]+);.*' "$r")"
cmp1 smc_wdt_refresh.rs EARLY_S "$(cap '^pub const EARLY_S: u32 = ([0-9_]+);.*' "$r")"
for c in INIT SET_TIMEOUT ENABLE PET GET_TIMELEFT; do
	cmp1 smc_wdt.rs "CMD_$c" "$(cap "^pub const CMD_$c: u64 = ([0-9_]+);.*" "$rs/smc_wdt.rs")"
done
grep -qE '^pub const MIN_TIMEOUT_S: u32 = EARLY_S \+ 1;' "$r" || mm "smc_wdt_refresh.rs: MIN_TIMEOUT_S is not EARLY_S + 1"
[ "$(( ${want[EARLY_S]:-0} + 1 ))" = "${want[MIN_TIMEOUT_S]:-}" ] || mm "fixture: MIN_TIMEOUT_S is not EARLY_S + 1"
cmp1 mp2_wdtr.h SMC_FUNC_ID "$(cap '^#define MP2_WDTR_FUNC_ID_DK +([0-9a-fA-Fx]+u?).*' "$hdr")"
cmp1 mp2_wdtr.h MIN_TIMEOUT_S "$(cap '^#define MP2_WDTR_MIN_TIMEOUT_S +([0-9]+u?).*' "$hdr")"
cmp1 mp2_wdtr.h EARLY_S "$(cap '^#define MP2_WDTR_EARLY_S +([0-9]+u?).*' "$hdr")"

# Upstream trees: re-extract by pattern, and the cited line must hold the value.
extract() { # key file
	case "$1" in
	SMC_FUNC_ID) cap '^CFG_WDT_SM_HANDLER_ID \?= *(0x[0-9a-fA-F]+).*' "$2";;
	CMD_*) cap "^#define SMCWD_${1#CMD_}[[:space:]]+([0-9]+).*" "$2";;
	TIMEOUT_S) cap '^[[:space:]]*timeout-sec = <([0-9]+)>;.*' "$2";;
	EARLY_S) cap '^#define IWDG_ETIMEOUT_SEC[[:space:]]+U\(([0-9]+)\).*' "$2";;
	esac
}
while read -r key val src; do
	case "$key" in ''|'#'*) continue;; esac
	tree=${src%%:*}
	rest=${src#*:}
	path=${rest%:*}
	line=${rest##*:}
	case "$tree" in optee) t=${ST_OPTEE_R31_TREE:-};; tfa) t=${ST_TFA_R31_TREE:-};; *) continue;; esac
	[ -n "$t" ] || continue
	f=$t/$path
	[ -r "$f" ] || { mm "$key: $tree:$path not found in $t"; continue; }
	got=$(extract "$key" "$f")
	if [ -z "$got" ]; then mm "$key: no value in $tree:$path"; continue; fi
	[ "$(norm "$got")" = "$(norm "$val")" ] || mm "$key: $tree:$path has $(norm "$got"), fixture has $(norm "$val")"
	sed -n "${line}p" "$f" > "${TMPDIR:-/tmp}/wdtline.$$"
	at=$(extract "$key" "${TMPDIR:-/tmp}/wdtline.$$")
	rm -f "${TMPDIR:-/tmp}/wdtline.$$"
	[ -n "$at" ] || mm "$key: line $line of $tree:$path no longer holds the value"
done < "$facts"
exit $bad
