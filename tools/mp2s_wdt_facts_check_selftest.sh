#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Selftest of tools/mp2s_wdt_facts_check.sh: the real tree must pass; mutated
# copies of the Rust source, the C header and the fixture, and synthetic
# upstream trees with a wrong value, must fail with MISMATCH. No board, no network.
# When ST_OPTEE_R31_TREE / ST_TFA_R31_TREE are set the real trees are checked too.
set -u
here=$(cd "$(dirname "$0")/.." && pwd)
chk=$here/tools/mp2s_wdt_facts_check.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/wdtfacts.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { echo "ok   $1"; }
no() { echo "FAIL $1"; fail=1; }

# expect NAME WANT_RC [GREP] -- env... : run the checker with the given env
run() { env "$@" bash "$chk" 2>&1; }

out=$(run MP2S_WDT_ROOT="$here"); rc=$?
if [ $rc -eq 0 ] && [ -z "$out" ]; then ok "real tree agrees with the fixture"; else no "real tree (rc=$rc): $out"; fi

# With ST_OPTEE_R31_TREE / ST_TFA_R31_TREE set, the run above already checked them.

bash "$chk" extra >/dev/null 2>&1; [ $? -eq 2 ] && ok "usage: extra argument exits 2" || no "extra argument not exit 2"
out=$(run MP2S_WDT_ROOT="$tmp/nonexistent"); rc=$?
[ $rc -eq 2 ] && ok "usage: unreadable source exits 2" || no "unreadable source rc=$rc"

mkcopy() { # dir
	local d=$1
	mkdir -p "$d/driver/rust/src/soc/stm32mp2" "$d/tools/probes/stm32mp257f_dk/mp2_wdt/src" "$d/tools/testdata/mp2s-wdt"
	cp "$here"/driver/rust/src/soc/stm32mp2/smc_wdt.rs "$here"/driver/rust/src/soc/stm32mp2/smc_wdt_refresh.rs "$d/driver/rust/src/soc/stm32mp2/"
	cp "$here/tools/probes/stm32mp257f_dk/mp2_wdt/src/mp2_wdtr.h" "$d/tools/probes/stm32mp257f_dk/mp2_wdt/src/"
	cp "$here/tools/testdata/mp2s-wdt/facts.txt" "$d/tools/testdata/mp2s-wdt/"
}
# mutate NAME FILE SED_EXPR WANT_GREP
mutate() {
	local name=$1 file=$2 expr=$3 want=$4 d=$tmp/m.$1
	mkcopy "$d"
	sed -i -E "$expr" "$d/$file"
	if cmp -s "$d/$file" "$here/$file"; then no "mutation $name changed nothing"; return; fi
	out=$(run MP2S_WDT_ROOT="$d"); rc=$?
	if [ $rc -eq 1 ] && grep -q "^MISMATCH: .*$want" <<<"$out"; then ok "mutated $name fails"; else no "mutated $name (rc=$rc): $out"; fi
}
R=driver/rust/src/soc/stm32mp2
H=tools/probes/stm32mp257f_dk/mp2_wdt/src/mp2_wdtr.h
F=tools/testdata/mp2s-wdt/facts.txt
mutate rust-func-id $R/smc_wdt_refresh.rs 's/(FUNC_ID_DK: u32 = )0xbc00_0000/\10xbc00_0001/' SMC_FUNC_ID
mutate rust-timeout $R/smc_wdt_refresh.rs 's/(TIMEOUT_DK_S: u32 = )32/\130/' TIMEOUT_S
mutate rust-early $R/smc_wdt_refresh.rs 's/(pub const EARLY_S: u32 = )5/\14/' EARLY_S
mutate rust-min $R/smc_wdt_refresh.rs 's/EARLY_S \+ 1;/EARLY_S;/' MIN_TIMEOUT_S
mutate rust-cmd-pet $R/smc_wdt.rs 's/(CMD_PET: u64 = )3/\17/' CMD_PET
mutate rust-cmd-enable $R/smc_wdt.rs 's/(CMD_ENABLE: u64 = )2/\19/' CMD_ENABLE
mutate hdr-func-id $H 's/0xbc000000u/0xb200005au/' SMC_FUNC_ID
mutate hdr-min $H 's/(MIN_TIMEOUT_S +)6u/\15u/' MIN_TIMEOUT_S
mutate hdr-early $H 's/(MP2_WDTR_EARLY_S +)5u/\13u/' EARLY_S
mutate fixture-timeout $F 's/^TIMEOUT_S 32 optee/TIMEOUT_S 30 optee/' TIMEOUT_S
mutate fixture-no-line $F 's/^(EARLY_S 5 optee:[^:]*):74/\1/' 'no upstream path:line'

# Synthetic upstream trees built from the fixture's own citations
mktree() { # dir  value-overrides: EARLY_S=4 ...
	local d=$1 key val src tree rest path line f
	shift
	mkdir -p "$d/optee" "$d/tfa"
	while read -r key val src; do
		case "$key" in ''|'#'*) continue;; esac
		tree=${src%%:*}; rest=${src#*:}; path=${rest%:*}; line=${rest##*:}
		[ "$tree" = derived ] && continue
		for o in "$@"; do [ "${o%%=*}" = "$key" ] && val=${o#*=}; done
		f=$d/$tree/$path
		mkdir -p "$(dirname "$f")"
		[ -e "$f" ] || : > "$f"
		while [ "$(wc -l < "$f")" -lt $((line - 1)) ]; do echo "/* pad */" >> "$f"; done
		case "$key" in
		SMC_FUNC_ID) echo "CFG_WDT_SM_HANDLER_ID ?= $val" >> "$f";;
		CMD_*) echo "#define SMCWD_${key#CMD_}	$val" >> "$f";;
		TIMEOUT_S) echo "	timeout-sec = <$val>;" >> "$f";;
		EARLY_S) echo "#define IWDG_ETIMEOUT_SEC	U($val)" >> "$f";;
		esac
	done < "$here/tools/testdata/mp2s-wdt/facts.txt"
}
mktree "$tmp/t.good"
out=$(run MP2S_WDT_ROOT="$here" ST_OPTEE_R31_TREE="$tmp/t.good/optee" ST_TFA_R31_TREE="$tmp/t.good/tfa"); rc=$?
[ $rc -eq 0 ] && ok "synthetic upstream trees agree" || no "synthetic good trees (rc=$rc): $out"
for o in SMC_FUNC_ID=0xbc000001 CMD_PET=9 EARLY_S=4; do
	mktree "$tmp/t.$o" "$o"
	out=$(run MP2S_WDT_ROOT="$here" ST_OPTEE_R31_TREE="$tmp/t.$o/optee" ST_TFA_R31_TREE="$tmp/t.$o/tfa"); rc=$?
	if [ $rc -eq 1 ] && grep -q "^MISMATCH: ${o%%=*}: " <<<"$out"; then ok "upstream $o fails"; else no "upstream $o (rc=$rc): $out"; fi
done
mktree "$tmp/t.tfa" TIMEOUT_S=30
out=$(run MP2S_WDT_ROOT="$here" ST_OPTEE_R31_TREE="$tmp/t.tfa/optee" ST_TFA_R31_TREE="$tmp/t.tfa/tfa"); rc=$?
if [ $rc -eq 1 ] && grep -q "^MISMATCH: TIMEOUT_S: " <<<"$out"; then ok "upstream TIMEOUT_S fails"; else no "upstream TIMEOUT_S (rc=$rc): $out"; fi
# a value that moved: the cited line no longer holds it
mktree "$tmp/t.moved"
sed -i '/IWDG_ETIMEOUT_SEC/d' "$tmp/t.moved/optee/core/drivers/stm32_iwdg.c"
{ echo "#define IWDG_ETIMEOUT_SEC	U(5)"; } >> "$tmp/t.moved/optee/core/drivers/stm32_iwdg.c"
sed -i '1i /* shifted */' "$tmp/t.moved/optee/core/drivers/stm32_iwdg.c"
out=$(run MP2S_WDT_ROOT="$here" ST_OPTEE_R31_TREE="$tmp/t.moved/optee" ST_TFA_R31_TREE="$tmp/t.moved/tfa"); rc=$?
if [ $rc -eq 1 ] && grep -q "^MISMATCH: EARLY_S: line 74" <<<"$out"; then ok "moved upstream line is reported"; else no "moved line (rc=$rc): $out"; fi

[ $fail -eq 0 ] && echo "mp2s_wdt_facts_check selftest: all ok" || echo "mp2s_wdt_facts_check selftest: FAILED"
exit $fail
