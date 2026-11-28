<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The lease route: a T6 U-Boot session through the lease's serial tools

When the farm's console server (the lease) holds the serial adapter's one
client slot, `prompt-reads.sh --go` and `prompt-go.sh --go` cannot open the
line (their exit 40; `console-check.sh` exit 6 or 7). This runbook is the
same session done by hand, as the Pi 4 sessions did it
(`tools/cts/log/2026-11-pi4-gles3-fad0558a-rerun/README.md`, "Method"): the
agent types through the lease's MCP tools `serial_status`, `serial_write`,
`serial_expect` and `serial_read`, and the repo's tools keep what they can:
the command list and what each answer must be (`prompt-reads.py plan`,
`prompt-go.py plan`), the staging and its undo (`stage.sh`), the verdict
(`t6-verdict.sh`), the summary, the redaction and the log copy. What the
wrappers do on the line (the echo check, the exit codes of a run) is done
here by the agent, by the rules below. `README.md`'s "Session runbook" and
"Board rules" still hold; this file replaces only how the console is
reached.

The board's address, the serial line and `T6_HAND` come from the environment
only and are never written into a file, a record or a commit.

## Limits (all of README "Board rules" applies)

- Staging is `/boot/yakogl` only, through `stage.sh`. The only U-Boot
  commands typed are the lines `plan` prints (the reads, the load, `go`,
  `reset`); never from memory, never anything else, `serial_send_break`
  included.
- Never write the eMMC's loader or boot area, U-Boot's `bootcmd` or its
  environment (no `saveenv`, no `setenv`), a kernel module, Tailscale or sshd.
- One reboot per image. A board that does not come back is reported as
  needing a person (the record directory's name, the last console lines) and
  is not retried, rebooted again or power-cycled. A run that fails stops the
  session at its first failure: record the cause, stop.

## Steps

Paths are from the checkout's root. `<img>` is the image directory,
`<probe>` the image's probe name (`t6_cold`, `t6_fw`, `t6_submit`,
`gl_test`, `triangle`, a dEQP row's label), `<fp>` the fingerprint file
(kept on this host), `<rec>` the record directory
`records/nanopc-t6/<YYYYMMDD-HHMM>-<name>-<sha8>/` (gitignored), made by
the session.

### 0. Preflight (host only)

```
T6_CONSOLE=lease T6_HAND=<name or none> driver/tools/uboot-t6/preflight.sh <img>
```

`T6_HAND` comes from the environment; `none` is the explicit "no hand
name". Exit 0 goes on; 1 a check printed `FAIL` (a host result, the board
was not touched: fix the host); 2 usage. `preflight.sh` at this commit reads
`T6_UART` in its check a and never opens it; on the lease route the line is
the lease's, so a `FAIL` on `T6_UART` alone is the check not yet knowing
`T6_CONSOLE=lease`, not a host fault. `console-check.sh` is not run (it
opens the line). Run `driver/tools/uboot-t6/selftest.sh` once if the tools
changed.

### 1. Stage, after the `/boot` fingerprint check

Copy each build's `zephyr.bin` to `<probe>.bin` first (the staged name is
the file's name), then

```
BOARD_HOST=<root@board> driver/tools/uboot-t6/stage.sh stage <img>/<probe>.bin <fp>
```

The first `stage` writes `<fp>` before anything changes on the board; later
calls go on only if `/boot` still matches it (`stage.sh check` does the
comparison alone). Exit 0 goes on; 1 the copy or a stock file differs; 2
usage; 3 refused, nothing written. Stage every probe the session runs
before the first reboot. For each, note its size (`stat -c %s`) and CRC-32
(`python3 -c 'import sys,zlib;print("%08x"%zlib.crc32(open(sys.argv[1],"rb").read()))' <file>`),
and print what must be typed:

```
python3 -I driver/tools/uboot-t6/prompt-go.py plan <probe>.bin <size> <crc>
python3 -I driver/tools/uboot-t6/prompt-reads.py plan      # a reads session
```

### 2. Note the offset, then reboot

1. `serial_status`: note its offset. Every later read starts from it, or
   from the last `next_offset`.
2. `ssh <root@board> 'cat /proc/sys/kernel/random/boot_id'`: keep the value.
3. `ssh <root@board> 'systemctl reboot'` (what `prompt-lib.sh` uses). The
   connection drops; that is expected.

Do not write to the line before U-Boot's autoboot line: Armbian logs in on
it and a byte then goes into a root shell.

### 3. Stop the autoboot

1. `serial_expect` for `Hit any key to stop autoboot`, `since` at the noted
   offset, looped at its 120 s cap with `since` at the last `next_offset`
   while the board boots (about 240 s in all).
2. `serial_write` one space, `newline:"none"`.

No autoboot line in time: nothing was written (the wrappers' exit 12). If
Linux boots instead (a login prompt, no U-Boot prompt) the autoboot was
missed (exit 4): undo (step 7), retry at most once. A second miss, or a
first miss at 12, is reported and stops.

### 4. Type the plan, one command at a time

For each line of the plan, in order (the text before the `#`):

1. `serial_write` the command, `newline:"none"` (no CR).
2. `serial_read` the echo. Only if it equals the command byte for byte,
   `serial_write` an empty text with `newline:"cr"`; otherwise never enter
   it: send ctrl-c (`escape` for `^C`), read the fresh prompt, retype. At
   most 3 tries per command; after the third write nothing more (the board
   stays at the prompt: needs a person, exit 5).
3. `serial_expect` for the prompt, `serial_read` the answer and compare it
   with the plan's `#` text (`filesize=<hex>`, `<size> bytes read`, the
   magic word `644d5241`, the `==> <crc>` line). The reads' answers are only
   recorded.

A load answer that differs in size, magic or CRC (exit 8): do not type
`go`; type `reset` the same way, and go to step 5. A command with no answer
or a lost prompt stops (exit 5). Before `go` there is nothing else to type
for a go run; the reads list ends with `reset`.

### 5. After `go` or `reset`: write nothing more

Once `go` or `reset` is entered (CR sent) do not write to the line again,
whatever comes: the image owns the UART, then Armbian does. The image
returns by PSCI SYSTEM_RESET or WDT0.

1. `serial_expect` for `login:` (Linux), looped at 120 s with `since` at the
   last `next_offset`, up to the image's wait (the probe's IMAGE_WAIT; a
   few minutes after `reset`) plus 180 s for Linux. `## Starting
   application` must appear after `go` (else exit 9: nothing more written).
2. `ssh <root@board> 'cat /proc/sys/kernel/random/boot_id'` and compare with
   step 2's. A new boot_id is Linux back even if the UART shows no login
   (the adapter can stop its stream in the kernel's output, issue #11874);
   the same one means the reboot did not happen. Neither a login nor a new
   boot_id: needs a person (exit 6 after `reset`, 10 after a started
   image); report, do not retry.

### 6. Record and judge

Save the console bytes of `serial_read` (the whole session, by offset) to
`<rec>/uart.log` with a script that decodes the escapes and refuses an
offset gap or `truncated` (never copy by hand), the typed lines to
`<rec>/typed.log`, the answers to `<rec>/answers.txt`, and what the image
printed after `go` (from `## Starting application` to the login) to
`<rec>/image.log`. Then:

```
T6_ROUTE=uboot driver/tools/kexec-zephyr/t6-verdict.sh <probe> <rec>/image.log
python3 -I tools/cts/t6-uboot-handover-summary.py <reads-record> <t6_gpu-record> [<t6_cold-record>]
tools/cts/t6-redact-check.sh <rec>
tools/cts/t6-uboot-log.sh <rec> <YYYY-MM-DD> <name> <sha8>
```

`t6-verdict.sh` exits 0 PASS, 1 FAIL, 2 usage; for `gl_test` and dEQP rows
the verdict is read from the `T6:A`/`T6:G` lines and the sample's summary
lines named in `prompt-go.py`'s header, and recorded as hand-judged. The
summary exits 0, 2 usage, 3 an address in the input, 4 a record unreadable.
`t6-redact-check.sh` exits 0 clean, 1 a finding (file, line, kind; fix the
copy by hand), 2 usage. `t6-uboot-log.sh` exits 0, 1 something survived
redaction, 2 usage or no text file, 3 destination exists. Nothing of
`records/` leaves unchecked.

`tools/cts/t6-uboot-rows.sh` (README step 8) opens the line itself and
cannot run through the lease: do the dEQP rows one at a time with steps 1
to 7, `<probe>` being the row's label, and only after `gl_test` passed.

### 7. Undo

```
BOARD_HOST=<root@board> driver/tools/uboot-t6/stage.sh undo <fp>
```

Run it when the session ends, passed or stopped (and after a missed
autoboot). It removes `/boot/yakogl`; exit 0, or 1 when a stock file differs
(named; the directory is removed all the same), 2 usage, 3 refused. If the
board is not back in Linux, do not undo: report it as needing a person.

## Exit codes of the run, as the wrappers give them

The wrappers' codes keep their meaning when the steps are done by hand:
0 done (PASS, Linux back); 4 autoboot missed; 5 stopped at the prompt
(power cut needed); 6 `reset` entered, no Linux; 7 U-Boot left the prompt
and started Linux; 8 an answer differed, `reset`, Linux back; 9 `go` not
answered; 10 image started, no Linux; 11 verdict FAIL, Linux back; 12 no
autoboot line. The wrappers' 1, 3, 40 and 41 have no meaning here.

## A person is needed when

- the run ends as 5, 6 or 10 (or 9 with no sign of Linux), or Linux is not
  back within step 5's waits;
- a lease tool fails, the console script reports a gap or a decode error,
  or an echo fails 3 times;
- anything on the board differs from what the tools print (a stock `/boot`
  file in `stage.sh`'s answer, an unexpected U-Boot answer).

Then: stop writing, report the record directory's name, the last console
lines and the code, leave the staging as it is, and do not retry.
