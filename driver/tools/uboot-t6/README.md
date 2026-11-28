<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# uboot-t6: the NanoPC-T6 at its U-Boot prompt

Roadmap stage 5 asks for a standalone boot from the T6's own storage: U-Boot
loads the Zephyr image from the eMMC's ext4 partition and starts it, no Linux
before it. This directory holds the tools for that route.

Status:

- The reads at the prompt are a tool (`prompt-reads.sh`) and ran on the board
  on 2026-11-12. What the prompt answered is in "What the prompt answered".
  The list now also reads WDT0's gates, its words and the CRU's global reset
  words ("The watchdog"); that part has not run.
- Starting an image from the prompt is a tool (`stage.sh`, `prompt-go.sh`;
  "The route") and ran on the board on 2026-11-24: t6_hello, t6_gpu, t6_mmu,
  t6_fw and t6_submit, then `t6_cold` and the triangle image on HDMI, each
  with the verdict PASS (`driver/docs/nanopc-t6-display.md`, step D9). The
  tool types the commands; U-Boot starting the image by itself is not done.
  The script draft
  `t6-zephyr.cmd` is gone: it needed `setexpr`, `icache` and `dcache`, which
  this build lacks.
- The 2026-11-25 sessions (plan #408) stopped before the prompt: the
  adapter's one client slot was held by the farm's serial console server at
  the adapter's stored 115200, not the console's 1500000, so `--go` exited
  40 (`busy`). A `--go` run needs that slot free, or the line opened at
  1500000 (docs/notes/nanopc-t6-uboot-handover.md, section 5, "Board
  sessions"). `console-check.sh` ("Before a session: the console") now
  finds either cause before anything is rebooted; it ran once on the
  board (uboot-s1b, 2026-11-26): exit 6, slot busy, the board read as a
  T6 with `/dev/ttyS2` at 1500000. No session has reached the prompt since
  2026-11-12; the farm's "never reboot" rule for the T6 and a missing
  `T6_HAND` are the next blockers.
- `stage.sh` stages a file under its own name: copy each build's
  `zephyr.bin` to `<probe>.bin` before `stage.sh stage`, and give
  `prompt-go.sh` that file.

## Before a session: the console

`console-check.sh` proves that the debug UART reads at 1500000 on this host
while Armbian runs, before `prompt-reads.sh --go` or `prompt-go.sh --go`
reboots anything. `preflight.sh` (host only) prints the command to run it
(its check e).

```sh
BOARD_HOST=root@<t6> T6_UART=<tty or rfc2217://<host>:<port>> \
  driver/tools/uboot-t6/console-check.sh
```

It reads the board over ssh (a NanoPC-T6, root, `/dev/ttyS2` at 1500000),
opens `T6_UART` the way the runners do (a local tty set to 1500000 raw with
no echo and read back; an `rfc2217://` address through
`../kexec-zephyr/t6-uart-rfc2217.py`, whose adapter must confirm 1500000),
writes one line, `#### console-check <16 random hex digits>`, to
`/dev/ttyS2` on the board over ssh, and must read that line back on
`T6_UART` within `CONSOLE_CHECK_READ_S` seconds (default 5). It never
writes to `T6_UART` (a root shell reads that line), takes no board lock,
reboots nothing, and prints no address.

Exit codes, one meaning each; a session goes on only after 0: 0 the line
read back verbatim; 1 this script or the RFC 2217 tool failed (tool exit
41); 2 usage, `BOARD_HOST` or `T6_UART` unset; 3 the board over ssh (no
answer, not a T6, not root, the line not written); 4 the board's
`/dev/ttyS2` not at 1500000; 5 `T6_UART` not a tty this user can use at
1500000 (or open by another process); 6 the adapter's client slot busy
(tool exit 40, "busy"); 7 the RFC 2217 session not opened otherwise (tool
exit 40); 8 the adapter did not confirm 1500000 (tool exit 42); 9 no byte
read; 10 bytes read but not the line (the console reaches this host at
another rate). Its self-test is `console-check-selftest.sh` (a fake board
and a fake adapter on a pty; a gate row).

## Session runbook

The T6 sessions from U-Boot (prompt reads, `t6_cold`, `t6_gpu`, `t6_fw`,
`t6_submit`; then `gl_test`, the triangle probe, one or two dEQP-GLES2 rows)
run the tools below in this order. Each step names its command, the exit
that lets the session go on, and the exits that stop it. The meanings are
those of the tools' own headers and self-tests; the longer account is in
"Before a session: the console", "Prompt reads", "The route" and "Staging
under /boot and the undo". Paths are from the checkout's root; `<dir>`,
`$T6_UART`, `$T6_HAND`, `$T6_RECORD_DIR` and `$BOARD_HOST` come from the
environment (never written into a file, a log or a commit). Stop at the
first stop code: record its cause, do not retry it.

| # | Command | Goes on | Stops |
|---|---|---|---|
| 1 | `driver/tools/uboot-t6/preflight.sh <image-dir>` (host only) | 0 | 1 a check printed `FAIL`; 2 usage |
| 2 | `BOARD_HOST=... T6_UART=... driver/tools/uboot-t6/console-check.sh` | 0 | 1 tool failed; 2 usage; 3 board over ssh; 4 `/dev/ttyS2` not at 1500000; 5 local tty unusable; 6 adapter slot busy; 7 RFC 2217 session not opened; 8 adapter did not confirm 1500000 (tool exit 42); 9 no byte read; 10 bytes read, not the line |
| 3 | `BOARD_HOST=... driver/tools/uboot-t6/stage.sh stage <dir>/<probe>.bin <dir>/t6-boot.fingerprint` | 0 | 1 the copy or a stock file differs; 2 usage or missing tool; 3 refused, nothing written |
| 4 | `BOARD_HOST=... T6_HAND=... T6_RECORD_DIR=... T6_UART=... driver/tools/uboot-t6/prompt-reads.sh --go` | 0 | 12 no autoboot line, nothing written; 4 autoboot missed, Linux booted; 5 stopped at the prompt, power cut needed; 6 `reset` entered, no sign of Linux; 7 a command made U-Boot reset the board; 1 board not back in Armbian; 2 usage; 3 refused, board not rebooted; 40 RFC 2217 session or line not set; 41 RFC 2217 tool failed in the run |
| 5 | `BOARD_HOST=... T6_HAND=... T6_RECORD_DIR=... T6_UART=... driver/tools/uboot-t6/prompt-go.sh <dir>/<probe>.bin <probe> --go` | 0 verdict PASS, Armbian back | 12, 4, 5, 6, 7 as step 4; 8 an answer differed, no `go`, Armbian back; 9 `go` not answered; 10 image started, no sign of Linux, needs a person; 11 verdict FAIL, Armbian back; 1, 2, 3, 40, 41 as step 4 |
| 6 | `T6_ROUTE=uboot driver/tools/kexec-zephyr/t6-verdict.sh <probe> <record>/image.log` | 0 PASS | 1 FAIL (n of m lines not ok); 2 usage |
| 7 | `python3 -I tools/cts/t6-uboot-handover-summary.py <reads-record> <t6_gpu-record> [<t6_cold-record>]` | 0 | 2 usage; 3 an address in the input; 4 a record unreadable |
| 8 | `tools/cts/t6-uboot-rows.sh <image-dir> <out-dir> <LABEL>... --go` | 0 | 2 usage or a refused label or image, nothing run; 3 stopped at a prompt-go code other than 0 or 11 (named in `rows.tsv`) |
| 9 | `python3 -I tools/cts/t6-uboot-deqp-compare.py --uboot <rows out-dir\|rows.tsv\|record dir> --armbian tools/cts/log/2026-11-24-t6-gles2-S?-8ff01055` | 0 | 2 bad input; 3 only with `--kexec` (a bad kexec file) |
| 10 | `BOARD_HOST=... driver/tools/uboot-t6/stage.sh undo <dir>/t6-boot.fingerprint` | 0 | 1 `/boot/yakogl` removed but a stock file differs (named); 2 usage; 3 refused |
| 11 | `tools/cts/t6-redact-check.sh <record>` | 0 | 1 a finding (file, line and kind, never the text); 2 usage or a missing file |
| 12 | `tools/cts/t6-uboot-log.sh <record> <YYYY-MM-DD> <name> <sha8>`, into `tools/cts/log/<date>-t6-uboot-<name>-<sha8>/` | 0 | 1 refused, something survived redaction (see step 12); 2 usage or no text file; 3 destination exists |

Notes on the steps:

1. A refusal here is a host-side result (a missing variable, module, sum or
   `COMMIT`), not a board result. Fix the host and run it again; the board
   has not been touched. Build every image from one origin/main commit.
   `preflight.sh <image-dir>` touches no board and no network and prints
   no environment value; one `ok` or `FAIL` line per check: (a) `T6_UART`
   (or `T6_CONSOLE=lease`: the lease's serial tools carry the console) and
   `T6_HAND` (or `T6_HAND=none`: no one at the power switch) are set; (b) python3 imports what `prompt-go.py` and
   `t6-uart-rfc2217.py` import; (c) the image directory (below); (d)
   `records/` is gitignored; (e) the `console-check.sh` command is printed,
   not run. Check (c) needs a local `origin/main` ref (`git fetch` is not
   run for it). Exit 0 all ok, 1 a `FAIL`, 2 usage (not a directory). Check (c)
   needs one `COMMIT` file in `<image-dir>` (one sha, an ancestor of the
   local `origin/main`; nothing is fetched) and a `SHA256SUMS` (sha256sum
   format) whose hashes match, in one of two layouts. Single: each other
   regular file is an image with a line in `SHA256SUMS`. Rows (the
   `tools/cts/t6-deqp-images.sh` directory that step 8 reads): each
   first-level `<LABEL>/` holding `zephyr/` has its image
   `<LABEL>/zephyr/zephyr.bin`, summed by a top-level `SHA256SUMS` (line
   `<LABEL>/zephyr/zephyr.bin`) or by one under the label (line
   `zephyr/zephyr.bin`); a label without `zephyr.bin`, or a top-level file
   other than `<LABEL>.built`, `<LABEL>.log` (the build log
   `t6-deqp-images.sh` writes beside its `.built`; accepted only with that
   `.built`), `SHA256SUMS` and `COMMIT`, is named; both
   layouts in one directory fail. A real
   `t6-deqp-images.sh <table> <img-dir> [LABEL...]` writes both at the top of
   `<img-dir>` when its builder exits 0: `SHA256SUMS` (one line
   `<LABEL>/zephyr/zephyr.bin` per label of that call whose `<LABEL>.built`
   says rc=0 and whose image is not empty; a failed or empty row is named on
   stderr and left out; a later selective build replaces the lines of its
   own labels and keeps every other label's line, and a call whose
   checkout HEAD differs from the directory's `COMMIT` is refused before
   building, exit 2: use a fresh directory) and `COMMIT` (the full hash of the checkout's HEAD, the
   commit built from; no git HEAD: a warning and no file). preflight.sh
   reads exactly these two: `COMMIT` must be an ancestor of the local
   `origin/main`, and each image's sha256 must match its `SHA256SUMS` line.
   `t6-deqp-images.sh --dry-run <table> <img-dir> [LABEL...]` (`--dry-run`
   must be the first argument) writes nothing, builds nothing and needs no
   environment: it prints `<LABEL> <filter>` per selected row, exit 0 on
   success, 2 on error.
2. Run it with Armbian up and before any reboot. Only 0 lets the session
   go on; an RFC 2217 exit 42 (code 8) stops it.
3. Stage before the first `--go`. The fingerprint file stays on this host
   and the first `stage` writes it before anything changes on the board.
   Copy each build's `zephyr.bin` to `<probe>.bin` first (the staged name is
   the file's name) and stage each probe image. Keep the fingerprint's path:
   step 10 needs it, and the undo comes first in any abort.
4. `prompt-reads.sh` without `--go` is the dry run and changes nothing.
5. One run per probe, in this order: `t6_cold`, `t6_gpu`, `t6_fw`,
   `t6_submit`, then `gl_test`, `triangle`; `deqp` for the dEQP rows is
   step 8's. Each has a dry run without `--go` first. Codes 5 and 10 leave
   the board at the prompt or running until its power is cut.
6. `prompt-go.sh` already writes `verdict.txt` (for `gl_test` and `deqp`
   `prompt-go.py` judges). Run `t6-verdict.sh` again on `image.log` only
   to judge a record apart; `<probe>` is one of `t6_hello`, `t6_gpu`,
   `t6_mmu`, `t6_fw`, `t6_submit`, `t6_cold`, `triangle`.
7. Run it with the prompt reads record and the `t6_gpu` go record: the
   hand-over lines (`gpu_clock_set` keep, move or refuse, the RK806 supply,
   PD_GPU, CRU) are printed by `t6_gpu`, `t6_fw` and `t6_submit` through
   `t6_z1.c`, not by `t6_cold`. Give it the `t6_cold` record too and the
   block adds the WDT0 gate and display lines. Go record directories are
   named `<stamp>-t6-uboot-go`, one per probe run; `record.txt` or your
   notes say which probe each holds. `T6_RECORD_DIR` should be the session
   directory `records/nanopc-t6/<YYYYMMDD-HHMM>-<name>-<sha8>/`.
8. Only after `gl_test` passed. 0 and 11 are answers with Armbian back and
   the next label runs; any other code stops, never retried.
9. Per-case outcomes of the U-Boot run against Armbian's 8ff01055 tree
   (`--armbian tools/cts/log/2026-11-24-t6-gles2-S?-8ff01055`). Omit
   `--kexec`: no per-case kexec baseline is in the tree, and the output then
   starts `kexec: no per-case baseline`. `--uboot` takes step 8's
   `<out-dir>` or its `rows.tsv`, or one `prompt-go.sh` record directory.
   Run it before the records are moved (step 12) or redacted: it reads
   `image.log` through the paths `rows.tsv` names.
10. Run it when the session ends, passed or stopped. A 1 names the stock
    file that changed (a kernel update); the directory is removed all the
    same.
11. Run it on the copy that is going into the tree; a finding is fixed in
    the copy by hand. Nothing of `records/` leaves unchecked.
12. Every text file of the record, `answers.txt` and `uart.log` included, is
    copied through `tools/cts/console-redact.py`, which replaces MAC
    addresses, U-Boot's `serial#` value and the UUIDs and GUIDs on `mmc
    part` output (`PARTUUID=` too), so a real reads record copies as it is,
    with no `--exclude`. A refusal (exit 1) names a file and a kind of
    something that survived (a UUID off partition output, say): fix the
    copy by hand, or leave that file out with `--exclude NAME`, and run it
    again.

### Board rules

- Never write the eMMC or the microSD's boot partitions or the loader area.
  `stage.sh` writes `/boot/yakogl` only.
- Never touch U-Boot's boot command or environment beyond the runner's own
  variables, with their undo printed first. The image is only in RAM; `reset`
  or the power switch returns to the stock boot.
- No kernel module is loaded or unloaded.
- Leave Tailscale and sshd alone.
- A board that does not come back (exit 1, 5 or 10, or 6 with no sign of
  Linux) is reported as needing a person, with the record directory's name,
  and not retried: no second `--go`, no extra reboot.

## Prompt reads

`prompt-reads.sh` reboots the board into U-Boot, stops autoboot on the debug
UART, enters a fixed list of read commands (`prompt-reads.py plan` prints
it), and leaves with `reset`; the board then boots Armbian by itself.

```sh
# dry run: prints the plan and the board's state, changes nothing
BOARD_HOST=root@<t6> driver/tools/uboot-t6/prompt-reads.sh
# the run
BOARD_HOST=root@<t6> T6_HAND=<who> T6_RECORD_DIR=<dir> T6_UART=<tty> \
  driver/tools/uboot-t6/prompt-reads.sh --go
```

`T6_UART` may also be `rfc2217://<host>:<port>`, the network serial adapter
on the debug UART: `--go` then runs through
`../kexec-zephyr/t6-uart-rfc2217.py`, which sets 1500000 8N1 and gives the
runner a pty (`../kexec-zephyr/README.md`, "The debug UART over RFC 2217";
exit 40 when the line cannot be set). This holds for `prompt-go.sh` too.

What it guarantees:

- Every command is a read. The list has no `mw`, no `bootm`, `booti` or `go`,
  no `mmc write`, `ext4write` or `saveenv`, and no `regulator` command.
- It types only after U-Boot's own autoboot line, and only at U-Boot's
  prompt. Armbian logs root in on this UART by itself, so a byte typed after
  the kernel starts would reach a root shell: the tool stops typing when
  U-Boot leaves the prompt (exit 7).
- It takes the board lock (`../kexec-zephyr/board_lock.sh`) and refuses, never
  waits, when another runner holds it.
- The QoS words are read only if `PWR_GATE` shows PD_GPU on, and `pmic dump`
  only if `dm tree` shows the PMIC already probed.
- The self-test (`prompt-reads.py --self-test`, a fake board on a pty, no
  hardware) runs before the board is touched and in the gate
  (`../kexec-zephyr/board_lock_selftest.sh`).

The record is `record.txt`, `uart.log`, `typed.log` and `answers.txt` under
`$T6_RECORD_DIR/<date>-t6-uboot-reads`. `printenv` prints the board's MAC
addresses and serial number, and `mmc part` a partition UUID: `answers.txt`
and `uart.log` are never copied into the tree, an issue or a pull request as
they are.

Exit codes: 0 done and Armbian is back; 1 the board did not come back; 2
usage; 3 refused before anything changed (the board was not rebooted); 12
no autoboot line after the reboot, nothing written; 4 autoboot missed; 5
stopped at the prompt; 6 `reset` entered, and no sign of Linux (no login on
the UART, no new boot_id over ssh: "When the UART stream stops"); 7 U-Boot
left the prompt and started Linux.

The run on 2026-11-12 (record `20261112-184729-t6-uboot-reads`): 27 commands
answered, Linux's login 19 s after `reset`, exit 0.

## What the prompt answered

U-Boot 2026.07_armbian, `bootdelay=1`, `bootcmd=bootflow scan -lb`, console
on the debug UART at 1500000 baud.

| Question | Answer |
|---|---|
| Which mmc device is the eMMC? | `mmc 0` (`mmc@fe2e0000`; `mmc 1` is `mmc@fe2c0000`). `mmc 0:1` is the ext4 root, with `/boot` on it. |
| Is the 256 MiB window free? | Yes. `bdinfo`: U-Boot's code, stack, heap and device tree are at 0xecbcd000 and above (`relocaddr` 0xefec1000, `fdt_blob` 0xedbd07e0); nothing is reserved in 0x10000000-0x1fffffff. |
| Load addresses | `kernel_addr_r=0x02000000`, `fdt_addr_r=0x12000000`, `scriptaddr=0x00c00000`, `loadaddr=0xc00800`. Only `fdt_addr_r` lies in the window; the route does not use it. |
| PD_GPU | Off, its idle request clear: `PWR_GATE` 0x0000fff9, `BUS_IDLE_REQ` 0, `BUS_IDLE_ACK` and `BUS_IDLE_ST` 0x00021fff, `REPAIR_STATUS` 0xfff70001. |
| The GPU clock | `CLKSEL_CON(158)` 0x00001f80: the SPLL, not the CPLL / 5 Linux leaves (0x1f24). `CLKSEL_CON(159)` 0, `CLKGATE_CON(66)` 0x00000004. The CPLL is at 1500 MHz in normal mode (`MODE_CON0` 0x00000545), so `gpu_clock_set` moves the GPU to CPLL / 5. |
| The RK806 | `dm tree` shows it probed, with all its regulators, before any PMIC command: U-Boot probes it on every boot. Whether DCDC1 is on was not read at the prompt (no `pmic` command); the image's hand-over report reads it over SPI2. |
| A watchdog | None: no watchdog device in `dm tree`, no `wdt` command. |
| Commands the build lacks | `setexpr`, `dcache`, `icache`, `clk`, `wdt`, `pmic`, `ext4write`, `hash`, `sha1sum`. |
| Commands the route needs | Present: `ext4size`, `ext4load`, `md`, `crc32`, `go`, `reset`. |

The read list keeps `clk dump`, `wdt list` and the `pmic` commands: "Unknown
command" is the measurement that this build lacks them, and another build may
have them.

## The route

1. The image goes on the eMMC's ext4 root as `/boot/yakogl/<image>.bin`
   (`stage.sh`, "Staging under /boot and the undo"). Nothing of the stock
   `/boot` is changed, and U-Boot's environment is never saved.
2. `prompt-go.sh` reboots the board, stops autoboot and enters the commands
   on the UART (`prompt-go.py`), one at a time, checking each answer before
   the next: `ext4size mmc 0:1 /boot/yakogl/<image>.bin` (no answer), `printenv
   filesize` (the host's size), `ext4load mmc 0:1 0x10000000 ...` (that many
   bytes read), `md.l 0x10000038 1` (the `ARM\x64` magic at byte 56), `crc32
   0x10000000 <size>` (the host's CRC-32), then `go 0x10000000`. After `go`
   nothing is typed again. Any other answer: `reset`, no `go`.
3. The image turns off the MMU and caches `go` leaves on (`T6:G`, "What the
   image does itself"), arms WDT0, prints its record on the same UART and
   resets at its end. `bootflow scan -lb` then boots Armbian: no one-shot
   variable is needed. `prompt-go.py` judges the record with
   `t6-verdict.sh <probe>` (`T6_ROUTE=uboot`), or for `gl_test` and `deqp`
   itself (below), and waits for Linux's login.

The probes `prompt-go.sh` takes: `t6_hello`, `t6_gpu`, `t6_mmu`, `t6_fw`,
`t6_submit`, `t6_cold` and `triangle`, judged by `t6-verdict.sh`'s table for
each; and `gl_test` (`driver/samples/zephyr/gl_test` built for `nanopc_t6`)
and `deqp` (a dEQP-GLES2 row image, `tools/cts/t6-deqp-images.sh`), which
`t6-verdict.sh` has no table for and `prompt-go.py` (`judge_sample`) judges
by their own summary lines, in order:

1. `T6:A`'s SCTLR_EL2 with M, C and I clear, and `T6:G`'s value as found
   with one of them set and its value read back with all three clear and
   equal to `T6:A`'s (the same checks as `t6-verdict.sh`'s uboot route);
2. `gl_test`: its one `gl_test: all: F of T cases failed (P passed; X timed
   out; ...)` line with F 0, X 0 and T above 0 (the 50/50 record: `gl_test:
   all: 0 of 50 cases failed (50 passed; 0 timed out; ...)`). `deqp`:
   `deqp: done: ...` and then `deqp: summary: ...`; the cases' outcomes are
   not the verdict, they are tallied later from `image.log`, as under
   kexec;
3. the board glue's `t6: <probe> done: PSCI SYSTEM_RESET at N ms (F feeds)`
   (`t6_reset_to_linux`, called by the sample's own end).

`verdict.txt` then has one `ok`, `differs` or `missing` line per expected
line and `VERDICT <probe> PASS` or `VERDICT <probe> FAIL (<n> of <m>)` last,
as `t6-verdict.sh` writes it, so the exit codes below mean the same for
every probe. The wait for the image is its own bound, from its marker:
WDT0's lifetime plus one period (or one period on a proof image), plus 60 s
for U-Boot; a `gl_test` image (fed for 300 s) waits 383 s and a dEQP row
image (600 s) 683 s.

No script file, no `mkimage`, no `source`: the checks are the runner's, on
the host, where they are tested without a board (`prompt-go.py
--self-test`, `selftest.sh`; both run in
`../kexec-zephyr/board_lock_selftest.sh`, a gate row).

```sh
# once per image: stage it (the fingerprint file stays on this host)
BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh stage <dir>/t6_hello.bin <dir>/t6-boot.fingerprint
# dry run: the image's checks, the staged copy's sha256, the commands; changes nothing
BOARD_HOST=root@<t6> driver/tools/uboot-t6/prompt-go.sh <dir>/t6_hello.bin t6_hello
# the run
BOARD_HOST=root@<t6> T6_HAND=<who> T6_RECORD_DIR=<dir> T6_UART=<tty> \
  driver/tools/uboot-t6/prompt-go.sh <dir>/t6_hello.bin t6_hello --go
# afterwards: take it away again
BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh undo <dir>/t6-boot.fingerprint
```

`prompt-go.sh` refuses an image without the watchdog marker, linked
elsewhere than 0x10000000, larger than the window, or without the `T6:G`
line (an image built before its entry coped with `go`), and an image whose
staged copy is not the same bytes. Its record is `record.txt`, `uart.log`,
`typed.log`, `answers.txt`, `image.log` (the UART from `go` to Linux's
login) and `verdict.txt` under `$T6_RECORD_DIR/<date>-t6-uboot-go`.

Exit codes (`prompt-go.py`'s, passed on by `prompt-go.sh`, which first asks
the board over ssh after a 6 or a 10): 0 the verdict
PASS and Armbian back; 12 no autoboot line after the reboot, nothing
written; 4 the autoboot missed, Linux booted; 5 stopped at the U-Boot prompt (an echo that never
matched, no answer, the prompt lost), the board stays there until its power
is cut; 6 an answer differed, `reset` entered, and no sign of Linux; 7 a
command made U-Boot reset the board; 8 an answer differed: no `go`, `reset`
entered, Armbian back; 9 `go` not answered (no "## Starting application");
10 the image started, and no sign of Linux: the board needs a hand; 11 the
verdict FAIL, Armbian back (for `gl_test` also a case failed or timed out,
for `deqp` no `deqp: done:` or no `deqp: summary:` line). "No sign of Linux" is no login on the UART and
no new boot_id over ssh ("When the UART stream stops"). `prompt-go.sh` adds
1 the board did not come back into Armbian otherwise, 2 usage, 3 refused before anything changed (only that: the board
was not rebooted), and with an RFC 2217 address 40 (the line could not be
set, nothing done) or 41 (that tool failed during the run). The two runners share their ssh, board-fact, UART and
reboot-and-wait code in `prompt-lib.sh`.

From `go` until the image arms WDT0 (`T6:A3`) nothing resets a board whose
image stopped: U-Boot runs no watchdog and Linux's is gone (the kexec route
has Linux's dw_wdt across the jump; this one has none). `T6_HAND` is
required for that reason.

Link address and window:
`driver/boards/friendlyarm/nanopc_t6/nanopc_t6.dts`'s `zephyr,sram`, 256 MiB at
0x10000000 (0x10000000-0x1fffffff). `driver/tools/kexec-zephyr/nanopc-t6.sh`'s
`WINDOW_BASE` and `WINDOW_SIZE`, `prompt-go.sh`'s `WINDOW_BASE` and
`WINDOW_SIZE` and `prompt-go.py`'s `LOAD` hold the same values, and
`driver/tools/kexec-zephyr/board_lock_selftest.sh` fails when one differs.

### When the UART stream stops

The network serial adapter on the debug UART can stop its stream to the
host for good after a large burst, and the kernel's boot output is one; a
new connection works again (issue #11874; the adapter's defect is
signal-slot/serial-over-tailnet#2). A login Armbian prints after the stop is
never read, although Linux is back. So the UART tools (`prompt-reads.py`,
`prompt-go.py`) say only "no Linux login on the UART" (their 6 and 10), and
the runner asks the board over ssh, as it does after every run
(`prompt-lib.sh`, `exit_when_back`):

- Armbian answers with a new boot_id: Linux is back. The exit is the code of
  the login seen (`prompt-go.sh`: 6 becomes 8, and 10 becomes 0 or 11 by the
  verdict; `prompt-reads.sh`: 6 becomes 0), and `record.txt` says so.
- No answer within 180 s: no sign of Linux. 6 or 10 stays the exit, and the
  board needs `T6_HAND`.

The tools do not wait their whole time for a login that cannot come (issue
#11943: the run of record `20261124-185856-t6-uboot-go` ended 574 s after
the reboot command, with Linux up after about 133 s). While its tool
listens, the runner asks the board for its boot_id every 5 s
(`prompt-lib.sh`, `reboot_and_wait_tool`), and on a new one it makes a file
the tool got as its last argument. A wait for the autoboot line or for
Linux's login ends when that file is there and the UART has been silent for
30 s. A UART that still delivers bytes is read to the login as before. The
exit codes do not change: 6 and 10 as above, and 12 when it was the autoboot
line that never came (the board booted Linux without the line reaching the
tool; nothing was written).

The record shows the hole: when a wait for the login ends without it, the
tool closes `uart.log` with one line that starts with `####` and gives the
time and the byte offset of the last byte read (the times are those of
`typed.log`, which says the same), and whether the wait ran its whole time
or ended on the runner's word. `image.log`, the verdict's input, keeps
only the bytes read. The Zephyr part of the record is read before the
kernel starts, so the verdict does not depend on the stop.

Nothing is written to the UART for this: after `go` or `reset` the tools
never write again (Armbian logs root in on this console by itself), and the
answer over ssh is a read of `/proc/sys/kernel/random/boot_id`. A second UART
connection to look for the login again was not chosen: it cannot see a login
printed before it opened, so ssh would still be needed to tell "no Linux"
from "a login not read", and it would add a reconnect on silence, a
heuristic, to `t6-uart-rfc2217.py` for a defect of the adapter.

## What the image does itself

- **MMU and caches.** U-Boot runs with the MMU, the data cache and the
  instruction cache on (common/board_r.c:110-113; arch/arm/cpu/armv8/
  cache_v8.c:817, :880-894, :1172-1175), and `go` calls the entry as a plain
  function, with no cleanup (cmd/boot.c:17-22, :41; the ARM override
  arch/arm/lib/cmd_boot.c is built only for ARMv7-M, arch/arm/lib/
  Makefile:35). Nothing from `ext4load` to the entry cleans the data cache
  (cmd/ext4.c:48-51; fs/fs.c:625-627, :845-865; fs/ext4/ext4fs.c:71-200).
  The file's sectors come by SDMA into DRAM (the T6 defconfig's
  `CONFIG_MMC_SDHCI_SDMA`, :64; drivers/mmc/sdhci.c:104-105, :179-183), but
  the CPU stores a sector read in part (fs/fs_internal.c:51, :69, :91), a
  hole (fs/ext4/ext4fs.c:188) and a block cache hit
  (drivers/block/blk-uclass.c:441-443) through its data cache, so those
  bytes can still be dirty at `go`. The other boards' route runs `dcache
  off; icache off` before `go` (`tools/uboot-netboot-lib.sh`); this build has
  neither command. Zephyr's `reset.S` clears only SCTLR.A (Zephyr
  `arch/arm64/core/reset.S:62-66`). So the image's first code, at Zephyr's
  earliest hook (`z_arm64_el2_plat_prep_c`, before any stack), turns the
  MMU and both caches off, then cleans and invalidates zephyr,sram to the
  point of coherency by address, then invalidates the instruction cache and
  the EL2 TLB, and prints `T6:G` with `SCTLR_EL2` as found and as read
  back after the turn-off (driver/rust/src/board/nanopc_t6_go.rs).
  `t6-verdict.sh` (`T6_ROUTE=uboot`) passes a record only when the value
  read back has M, C and I clear and equals `T6:A`'s `SCTLR_EL2`. Entered
  at EL1 (not this U-Boot's case) the entry turns nothing off and prints
  `T6:G EL1` when the MMU or a cache is on there.
  Under kexec the entry finds them off and does nothing (Z1 measured
  `SCTLR_EL2` 0x30400800).
- **The watchdog.** U-Boot runs none. The image arms WDT0 itself, as after
  kexec: `yd_dw_wdt_arm` opens WDT0's two gates (CLKGATE_CON(15) bits 0
  and 1) if they are closed, checks the block's identity and refuses (then
  PSCI resets the SoC) when it does not answer. What the words are under
  U-Boot is not measured: the read list now has CLKGATE_CON(15)
  (0xfd7c083c), GLB_RST_ST (0xfd7c0c04) and GLB_RST_CON (0xfd7c0c10), and,
  only when both gates read open, WDT0's CR and TORR and its
  COMP_PARAM_1, COMP_VERSION and COMP_TYPE. GLB_RST_CON decides whether a
  WDT0 timeout resets the SoC from this state; WDT0's soft reset line is not
  read (no source on this host names its CRU bit).
  Decoding of these reads (which `CLKGATE_CON(15)` value lets WDT0 count,
  what the code does and does not say about `GLB_RST_CON` and `GLB_RST_ST`)
  and the rule for calling a WDT0 reset proven: docs/notes/nanopc-t6-uboot-handover.md,
  section 6. `prompt-go` has no exit code of its own for a WDT0 return: an
  image that ends by WDT0 or by PSCI exits 0 on a PASS verdict with Armbian
  back, 11 on FAIL, 10 when no sign of Linux follows ("Exit codes"). For the
  gl_test and deqp kinds the judge takes the PSCI line or, with none, the next
  U-Boot banner (`DDR V...` or `U-Boot N...`) after the summary, as
  `t6-verdict.sh` does; verdict.txt names the route (`route: psci|wdt0`).
- **The GPU.** PD_GPU off and the GPU clock on the SPLL are both states
  `gpu_up` handles (docs/notes/nanopc-t6-uboot-handover.md, sections 1 and
  2); neither has run on the board from U-Boot's state.

## go, not booti

The image does carry an arm64 Image header (`ARM\x64` at byte 56: Zephyr's
`arch/arm64/core/header.S`, text_offset 0, flags `HEADER_PGSIZE << 1`, so
bit 3 clear), yet `go` is the right command (U-Boot v2026.07, commit
ece349ad):

- `booti` (cmd/booti.c:112 do_booti, :23 booti_start) calls `booti_setup`
  (cmd/booti.c:77). That moves the image when bit 3 of the flags is clear:
  `dst` is the first DRAM bank's start (arch/arm/lib/image.c:69-72) and the
  target is `ALIGN(dst, SZ_2M) + text_offset` (arch/arm/lib/image.c:74), then
  `memmove` there (cmd/booti.c:83-85). A Zephyr image runs only at its link
  address, so it would be moved off it. A zero `image_size` is read as 16 MiB
  (arch/arm/lib/image.c:52-55); the Zephyr header carries `_flash_used`.
- `booti` also looks for ramdisk and device tree (cmd/booti.c:105-106) and
  passes the device tree in x0 (arch/arm/lib/bootm.c:290-291, via
  `armv8_switch_to_el2`); these images take none.
- `go` (cmd/boot.c:24-48 do_go, :41 do_go_exec; weak default at :18-22)
  calls the address in place with `argc`, `argv`, no relocation, and without
  `cleanup_before_linux`/`bootm_final` (compare arch/arm/lib/bootm.c:257-270,
  where booti's path does them).
- `ext4load` is `U_BOOT_CMD(ext4load ...)` at cmd/ext4.c:86 (do_ext4_load
  :48); the load sets `filesize` (fs/fs.c:865), `ext4size` sets it too
  (fs/fs.c:787).

U-Boot is GPL: the above are facts and file:line only.

## Staging under /boot and the undo

`stage.sh`, from this host over ssh as root. The fingerprint file stays on
this host: the sha256 of every file under `/boot` but `/boot/yakogl`, sorted
by name. `stage` writes it before anything changes on the board (or, when it
exists, goes on only if `/boot` still matches it); every command compares
`/boot` with it again after its own change.

```sh
BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh stage <image>.bin <fingerprint>
BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh check <image>.bin <fingerprint>
BOARD_HOST=root@<t6> driver/tools/uboot-t6/stage.sh undo <fingerprint>
```

What it writes on the board, and nothing else: the directory `/boot/yakogl`
(0755), one file per image in it (0644, written to a `.part` file in the
same directory and renamed), `sync`; `undo` is `rm -rf -- /boot/yakogl`. It
never writes another file of `/boot`, a block device, the eMMC's loader area
or U-Boot's environment, and loads no kernel module. Exit: 0 done and
`/boot`'s other files match the fingerprint; 1 the copy or a stock file
differs (named); 2 usage; 3 refused before anything was written.

The fingerprint holds the sha256 of every regular file and the target of
every symbolic link under `/boot` (Armbian's `Image`, `uInitrd` and `dtb`
are links). `stage` and `check` refuse a `/boot/yakogl` that is a symbolic
link or holds one: a write through it would leave `/boot/yakogl`. `undo`
removes `/boot/yakogl` even when a stock file changed since `stage` (a
kernel update does that): it names the change and exits 1 after the
removal; `rm -rf` removes a link, never what it points to.

The image is only in RAM after `ext4load`: `reset` or the power switch
returns to the stock `bootflow scan -lb`, which boots Armbian. A directory
`/boot/yakogl` is the only change on the partition, and `undo` removes it
and says "stock /boot restored" only when every other file matches the
fingerprint.
