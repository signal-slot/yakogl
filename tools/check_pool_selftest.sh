#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Pins tools/check.sh's row pool (--jobs). The same rows give the same log,
# line for line, with one job and with four: the order is the script's, a
# warning and a failure are red, a gap is counted and is not a pass. And a
# row that reads what an earlier row writes (`uses w:NAME` / `r:NAME`) waits
# for it; the control without the `uses` lines shows the reader fails when
# it does not wait, so the pass above is the lane's. And the compiler slots
# (the jobserver) bound the rows at once: with one slot two rows that would
# overlap do not, and the control with many slots shows the probe sees an
# overlap when there is one. And the jobserver's own failure: its FIFO is not
# under the checkout (a `git clean` of the next job removed it from under an
# orphaned gate, whose cargo then stored rustc's "failed to connect to
# jobserver" in the shared target directory, where every later build of the
# unit replays it), and a diagnostic of that kind that cargo stored earlier is
# removed before the rows run -- tested on the file a real cargo wrote, and the
# live case stays red (a probe whose jobserver cannot be opened).
set -u
# grep, sort and the tools the rows call print localized messages: compare C.
export LC_ALL=C LANG=C LANGUAGE=C
cd "$(dirname "$0")/.." || exit 2
fails=0
checks=0
mkdir -p build/check
dir=$(mktemp -d build/check/pool-selftest.XXXXXX) || exit 2
trap 'rm -rf "$dir"' EXIT
expect() {
  local label=$1 pattern=$2 out=$3
  checks=$((checks + 1))
  printf '%s\n' "$out" | grep -qE "$pattern" || {
    echo "FAIL $label: no line matches '$pattern'"
    printf '%s\n' "$out" | sed 's/^/    /' | tail -15
    fails=$((fails + 1))
  }
}
# A demo run has as many compiler slots as cores unless the caller says
# otherwise. check.sh takes a slot per row, so on a host with little free
# memory (runner C: 6 GB available = 1 slot of 3072 MB) the rows run one at a
# time, the reader of the control below runs after the writer, and the control
# cannot tell the lane from the slots (CI run 39100532612).
demo() {
  rm -f "$dir/lane" "$dir"/slot.*
  YAKOGL_CHECK_MEM_PER_COMPILER_MB=${YAKOGL_CHECK_MEM_PER_COMPILER_MB-1} \
  env -u YAKOGL_CHECK_JOBS -u MAKEFLAGS -u CARGO_MAKEFLAGS tools/check.sh --jobs "$1" --pool-demo "$dir" ${2-} 2>&1 |
    sed 's/\x1b\[[0-9;]*m//g'
}
# The log without the times, which differ between runs.
untimed() { printf '%s\n' "$1" | sed -E 's/ \[[0-9]+s\]$//; s/ in [0-9]+s;/;/; /compiler slot/d'; }
one=$(demo 1)
four=$(demo 4)
for jobs in 1 4; do
  out=$one
  [ "$jobs" = 4 ] && out=$four
  expect "jobs $jobs: the counts" '^check.sh: 5 ok, 2 failed, 1 gap\(s\)' "$out"
  expect "jobs $jobs: a warning is red" '^FAIL  demo    warns: 1 warning\(s\)' "$out"
  expect "jobs $jobs: a failure is red, its output under it" '^demo output$' "$out"
  expect "jobs $jobs: the reader waited for the writer" '^ok    demo    reads the lane.s file' "$out"
  expect "jobs $jobs: the gate is red" '^check.sh: fix the above' "$out"
done
checks=$((checks + 1))
if [ "$(untimed "$one" | grep -v whole.output)" != "$(untimed "$four" | grep -v whole.output)" ]; then
  echo "FAIL the log with four jobs is not the log with one:"
  diff <(untimed "$one") <(untimed "$four") | head -20
  fails=$((fails + 1))
fi
out=$(demo 4 nolanes)
expect "control: without the lane, the reader does not wait" '^FAIL  demo    reads the lane.s file' "$out"
out=$(YAKOGL_DEMO_AT_ONCE=1 YAKOGL_CHECK_MEM_PER_COMPILER_MB=999999999 demo 4)
expect "one slot: the slot line" '^check.sh: 1 compiler slot\(s\)' "$out"
expect "one slot: slot a ran alone" '^ok    demo    slot a' "$out"
expect "one slot: slot b ran alone" '^ok    demo    slot b' "$out"
out=$(YAKOGL_DEMO_AT_ONCE=1 YAKOGL_CHECK_MEM_PER_COMPILER_MB=1 demo 4)
expect "control: with many slots the probe sees two rows at once" '^2 at once$' "$out"
# The jobserver's FIFO is outside the checkout, and a cargo that cargo starts
# (an external subcommand, as nextest is) can open it.
bin="$dir/bin"
mkdir -p "$bin"
cat > "$bin/cargo-jsprobe" <<'PROBE'
#!/bin/sh
case "$CARGO_MAKEFLAGS" in
*"fifo:$PROBE_CHECKOUT/"*) echo "the jobserver is inside the checkout: $CARGO_MAKEFLAGS"; exit 1 ;;
*--jobserver-auth=fifo:/*) ;;
*) [ -n "${PROBE_ANY_FORM-}" ] || { echo "no jobserver path in: $CARGO_MAKEFLAGS"; exit 1; } ;;
esac
exec cargo metadata --no-deps --offline --format-version 1 --manifest-path "$PROBE_CHECKOUT/Cargo.toml" > /dev/null
PROBE
chmod +x "$bin/cargo-jsprobe"
probe() {
  env -u YAKOGL_CHECK_JOBS -u MAKEFLAGS -u CARGO_MAKEFLAGS PROBE_CHECKOUT="$PWD" PATH="$bin:$PATH" \
    YAKOGL_DEMO_ROW_NAME="nested cargo" YAKOGL_DEMO_ROW="$1" CARGO_TARGET_DIR="${2:-$dir/target}" \
    tools/check.sh --jobs 4 --pool-demo "$dir" 2>&1 | sed 's/\x1b\[[0-9;]*m//g'
}
out=$(probe 'cargo jsprobe')
expect "jobserver: a nested cargo opens it, and it is not under the checkout" '^ok    demo    nested cargo' "$out"
# The cached diagnostic: made by a real cargo (the build script removes the
# FIFO when it runs, so the build script's own rustc, started before it, opens
# it and the lib's rustc, started after it, cannot -- one unit stores the
# warning, however loaded the host; a timer did not, on a busy gate), then seen by
# check.sh before the rows. The same unit is built again with nothing changed
# and a clean environment: cargo prints the stored warning (the known positive
# of the pattern the purge searches for).
abs=$PWD/$dir
mkdir -p "$dir/crate/src"
printf '[package]\nname = "jsprobe"\nversion = "0.0.0"\nedition = "2021"\n\n[workspace]\n' > "$dir/crate/Cargo.toml"
printf 'fn main() { if let Some(p) = std::env::var_os("JSPROBE_FIFO") { let _ = std::fs::remove_file(p); } }\n' \
  > "$dir/crate/build.rs"
: > "$dir/crate/src/lib.rs"
# The live case stays red: a row whose rustc cannot open its jobserver. Made
# the way the known positive below makes it (a FIFO there when cargo starts,
# gone when the lib's rustc opens it), in a copy of the crate and a target
# directory of their own. A jobserver path that never existed does not do it:
# cargo then runs a jobserver of its own and its rustc never sees the path,
# and a nested `cargo metadata` starts no rustc at all, so that control was
# red on one run and green on the next (cargo 1.94.0-nightly 2025-12-13).
ctl=$PWD/$dir/control
cp -r "$dir/crate" "$ctl"
out=$(probe "mkfifo '$ctl/js' && exec 7<> '$ctl/js' && printf '++' >&7 && cd '$ctl' && env -u RUSTC_WRAPPER \
  JSPROBE_FIFO='$ctl/js' MAKEFLAGS='-j2 --jobserver-auth=fifo:$ctl/js' CARGO_MAKEFLAGS='-j2 --jobserver-auth=fifo:$ctl/js' \
  cargo build" "$ctl/target")
expect "control: the row sees a jobserver that cannot be opened" '^FAIL  demo    nested cargo: [0-9]+ warning' "$out"
expect "control: it is the jobserver's warning" 'failed to connect to jobserver' "$out"
mkfifo "$abs/js"
exec 8<> "$abs/js"
printf '+++' >&8
first=$(cd "$dir/crate" && env -u RUSTC_WRAPPER CARGO_TARGET_DIR="$abs/target" MAKEFLAGS="-j4 --jobserver-auth=fifo:$abs/js" \
  CARGO_MAKEFLAGS="-j4 --jobserver-auth=fifo:$abs/js" JSPROBE_FIFO="$abs/js" cargo build 2>&1)
exec 8>&-
checks=$((checks + 1))
if ! printf '%s\n' "$first" | grep -q 'failed to connect to jobserver'; then
  echo "FAIL known positive: a build whose FIFO went away did not print the warning"
  printf '%s\n' "$first" | sed 's/^/    /' | tail -8
  fails=$((fails + 1))
fi
again=$(cd "$dir/crate" && env -u RUSTC_WRAPPER -u MAKEFLAGS -u CARGO_MAKEFLAGS CARGO_TARGET_DIR="$abs/target" cargo build 2>&1)
expect "known positive: cargo replays the stored warning to a clean build" 'failed to connect to jobserver' "$again"
# Where cargo keeps it: .fingerprint/<unit>/, or <pkg>/<hash>/fingerprint/ in
# the build-dir layout of Cargo 1.100.0 and later. Other units may keep one
# too: a build script's holds an unused_extern record, and the warning as
# well when its compiler also started after the FIFO went (seen with the
# 2026-10 nightly under a loaded gate). The one meant is the library's file
# with the warning; the purge counts every file that has it.
holders=$(find "$dir/target" \( -path '*/.fingerprint/*' -o -path '*/fingerprint/*' \) -name 'output-*' -type f \
  -exec grep -l -F 'failed to connect to jobserver' {} +)
nholders=$(printf '%s\n' "$holders" | grep -c .)
stored=$(printf '%s\n' "$holders" | grep '/output-lib-jsprobe$')
checks=$((checks + 1))
[ "$(printf '%s\n' "$stored" | grep -c .)" -eq 1 ] || { echo "FAIL known positive: expected the library's stored output file, found: $holders"; fails=$((fails + 1)); }
printf '{"message":"unused variable: x","level":"warning"}\n' >> "$stored"
# A diagnostic that only contains the words in a source snippet is not the jobserver's.
printf '{"message":"unused variable: y","spans":[{"text":"failed to connect to jobserver"}],"level":"warning"}\n' >> "$stored"
out=$(probe 'true')
expect "purge: it names the file it edits" "^check.sh: purged the cached jobserver diagnostic from .*/output-" "$out"
expect "purge: it says how many files it cleaned" "^check.sh: removed cached 'failed to connect to jobserver' diagnostics of an earlier build from $nholders file" "$out"
checks=$((checks + 1))
if grep -q '"message":"failed to connect to jobserver' "$stored"; then
  echo "FAIL purge: the stored warning is still in $stored"
  fails=$((fails + 1))
fi
checks=$((checks + 1))
grep -q 'unused variable: x' "$stored" || { echo "FAIL purge: it removed a diagnostic that is not about the jobserver"; fails=$((fails + 1)); }
checks=$((checks + 1))
grep -q '"text":"failed to connect to jobserver"' "$stored" || { echo "FAIL purge: it removed a diagnostic whose snippet only contains the words"; fails=$((fails + 1)); }
again=$(cd "$dir/crate" && env -u RUSTC_WRAPPER -u MAKEFLAGS -u CARGO_MAKEFLAGS CARGO_TARGET_DIR="$abs/target" cargo build 2>&1)
checks=$((checks + 1))
if printf '%s\n' "$again" | grep -q '"message":"failed to connect to jobserver'; then
  echo "FAIL purge: a clean build still prints the warning"
  fails=$((fails + 1))
fi
# A nested check.sh (a jobserver in its environment: it runs inside a row of an
# outer run) does not purge: the outer run owns the target directory's files
# and its sibling rows' cargo writes them.
cp "$stored" "$dir/stored.keep"
printf '{"message":"failed to connect to jobserver from environment variable `CARGO_MAKEFLAGS`","level":"warning"}\n' >> "$stored"
cp "$stored" "$dir/stored.nested"
out=$(env -u YAKOGL_CHECK_JOBS MAKEFLAGS="-j2 --jobserver-auth=fifo:$abs/nonexistent" CARGO_MAKEFLAGS="-j2 --jobserver-auth=fifo:$abs/nonexistent" \
  CARGO_TARGET_DIR="$abs/target" tools/check.sh --jobs 4 --pool-demo "$dir" 2>&1 | sed 's/\x1b\[[0-9;]*m//g')
checks=$((checks + 1))
if printf '%s\n' "$out" | grep -q 'purged the cached'; then
  echo "FAIL nested: a check.sh inside a row purged"
  fails=$((fails + 1))
fi
checks=$((checks + 1))
cmp -s "$stored" "$dir/stored.nested" || { echo "FAIL nested: a check.sh inside a row edited the stored output"; fails=$((fails + 1)); }
# The sweep: a directory whose lock is held is not swept, whatever its pid
# says; one whose lock is free is. (Pid 999999999 is not a live process.)
mkdir -p "$dir/tmp/yakogl-check-js.999999999.held"
: > "$dir/tmp/yakogl-check-js.999999999.held/lock"
exec 6> "$dir/tmp/yakogl-check-js.999999999.held/lock"
flock -n 6
out=$(TMPDIR="$abs/tmp" probe 'true')
checks=$((checks + 1))
[ -d "$dir/tmp/yakogl-check-js.999999999.held" ] || { echo "FAIL sweep: a directory with a held lock was removed"; fails=$((fails + 1)); }
exec 6>&-
out=$(TMPDIR="$abs/tmp" probe 'true')
checks=$((checks + 1))
[ ! -d "$dir/tmp/yakogl-check-js.999999999.held" ] || { echo "FAIL sweep: a directory with a free lock was not removed"; fails=$((fails + 1)); }
PLANNED=32
echo "check_pool_selftest: $checks checks run (planned $PLANNED), $fails failed"
[ "$checks" -eq "$PLANNED" ] || { echo "FAIL the planned count $PLANNED is not the count run $checks"; fails=$((fails + 1)); }
[ "$fails" -eq 0 ]
