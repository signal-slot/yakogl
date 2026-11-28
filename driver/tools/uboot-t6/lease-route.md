<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The lease route: a T6 U-Boot session through the lease's serial tools

When the farm's console server (the lease) holds the serial adapter's one
client slot, `prompt-reads.sh --go` and `prompt-go.sh --go` cannot open the
line (their exit 40). This runbook is the same session done by hand, as the
Pi 4 sessions did it (`tools/cts/log/2026-11-pi4-gles3-fad0558a-rerun/README.md`,
"Method"): the agent types through the lease's MCP tools `serial_status`,
`serial_write`, `serial_expect` and `serial_read`; the repo's code keeps every
decision. The README's "Session runbook" and "Board rules" still hold; this
file replaces only how the console is reached.

Who decides what:

| Question | Who answers |
|---|---|
| what may be typed next | `prompt-reads.py next` (reads) or `prompt-go.py check` (go runs), on the transcript so far |
| whether an answer lets the run go on | `prompt-go.py check`, same transcript |
| the record and the verdict, with `run`'s exit code | `prompt-reads.py record` or `prompt-go.py judge` on the final transcript |
| the transcript | `tools/cts/t6-lease-transcript.py`, from the bytes `serial_read` returned |

The agent types nothing the code did not print, and never from memory. The
board's address, the serial line and `T6_HAND` come from the environment only
and are never written into a file, a record or a commit.

## Limits (all of README "Board rules" applies)

- Staging is `/boot/yakogl` only, through `stage.sh`. The only U-Boot
  commands are those the tools print (the reads, the load, `go`, `reset`).
- Never write the eMMC's loader or boot area, U-Boot's `bootcmd` or its
  environment (no `saveenv`, no `setenv` beyond what a tool prints), a kernel
  module, Tailscale or sshd.
- One reboot per session run. A stuck board is reported (exit 5, 6, 9 or 10
  with no sign of Linux): name the record directory, say it needs a person,
  and do not retry, reboot again or power-cycle. The same for a run that
  stops at its first failure: record the cause, stop.

## Steps

Paths are from the checkout's root. `<img>` is the image directory,
`<probe>` the image's probe name, `<fp>` the fingerprint file (kept on this
host), `<rec>` the record directory
`records/nanopc-t6/<YYYYMMDD-HHMM>-<name>-<sha8>/` (gitignored).

### 0. Preflight (host only)

```
T6_CONSOLE=lease T6_HAND=<name or none> driver/tools/uboot-t6/preflight.sh <img>
```

`T6_HAND` comes from the environment; `none` is the explicit "no hand
name". Exit 0 lets the session go on; 1 a check printed `FAIL` (a host
result: the board was not touched, fix the host); 2 usage. With
`T6_CONSOLE=lease` no serial line variable is needed. Also run
`driver/tools/uboot-t6/selftest.sh` once if the tools changed.

### 1. Stage, after the `/boot` fingerprint check

```
BOARD_HOST=<root@board> driver/tools/uboot-t6/stage.sh stage <img>/<probe>.bin <fp>
```

The first `stage` writes `<fp>` before anything changes on the board; later
calls go on only if `/boot` still matches it (`stage.sh check <img>/<probe>.bin <fp>`
does the comparison alone). Exit 0 goes on; 1 the copy or a stock file
differs; 2 usage; 3 refused, nothing written. Stage every probe the session
runs, once, before the first reboot. Copy each `zephyr.bin` to `<probe>.bin`
first: the staged name is the file's name.

### 2. Note the offset, then reboot

1. `serial_status`: note its offset (the console's history is empty at lease
   start; every later read starts from this offset, or from the last
   `next_offset`).
2. Over ssh: `ssh <root@board> 'cat /proc/sys/kernel/random/boot_id'` and
   keep the value.
3. Over ssh: `ssh <root@board> 'systemctl reboot'` (the command `prompt-lib.sh`
   uses). The connection drops; that is expected.

Do not write to the serial line before U-Boot's autoboot line: Armbian logs
in on it, and a byte then goes into a root shell.

### 3. Stop the autoboot

1. `serial_expect` with the pattern `Hit any key to stop autoboot`, `since`
   set to the noted offset. The cap is 120 s; loop it with `since` at the
   last `next_offset` while the board boots.
2. `serial_write` with one space, `newline:"none"` (no CR, no LF).

If the pattern never comes: nothing was written, run exit 12; report and
stop. If Linux boots instead (a login prompt, no U-Boot prompt), that is
run's exit 4: the autoboot was missed. Undo the staging (step 7), and retry
the session once at most; a second miss is reported and stops.

### 4. Type what the code prints, one command at a time

Build the transcript so far and ask the code. Reads session:

```
python3 -I tools/cts/t6-lease-transcript.py <serial_read output...> > <rec>/transcript.txt
python3 -I driver/tools/uboot-t6/prompt-reads.py next <rec>/transcript.txt
```

Go session (an image run): the same transcript, then

```
python3 -I driver/tools/uboot-t6/prompt-go.py check <rec>/transcript.txt ...
```

(`...` is the arguments `prompt-go.py` prints for `check` with no
arguments: the image's NAME, SIZE and CRC as `prompt-go.py plan` takes them;
the probe is the one staged.) `serial_read` is called with `since` at the
last `next_offset`, and the reads are concatenated by `next_offset`: the
transcript tool decodes the `\xNN` escapes, and handles `truncated` and
offset jumps. If it reports a gap, stop and report; do not guess.

For each command `next` or `check` prints:

1. `serial_write` the command with `newline:"none"`: no CR.
2. `serial_read` the echo, rebuild the transcript.
3. Only if the echo equals the command exactly, `serial_write` a single CR
   (`newline:"cr"`, empty text) to enter it. Wait for the answer
   (`serial_expect` for the prompt), rebuild the transcript and ask again.
4. If the echo differs in any byte, never enter it: send ctrl-c
   (`serial_write` with `escape` for `^C`), read the fresh prompt, and retype
   the command. At most 3 tries per command; after the third, stop writing
   (the board stays at the prompt: exit 5, needs a person).

When `check` says an answer differs (size, load, magic or CRC), it prints
`reset` instead of `go`: type `reset` the same way and the run ends as exit
8. When `next` or `check` prints nothing more to type, go on to step 5.

Never type a command the code did not print. `serial_send_break` is not
used.

### 5. After `go` or `reset`: write nothing more

Once `go` or `reset` has been entered (CR sent), do not write to the line
again, whatever comes: the image owns the UART, then Armbian does.

1. `serial_expect` for Linux's login (`login:`), looped at 120 s with `since`
   at the last `next_offset`, up to the image's wait (the plan's IMAGE_WAIT
   for a go run; a few minutes for `reset`), plus 180 s for Linux.
2. Over ssh: `ssh <root@board> 'cat /proc/sys/kernel/random/boot_id'` and
   compare with the value of step 2. A new boot_id is Linux back, even if
   the UART shows no login (the serial adapter can stop its stream in the
   kernel's output, issue #11874). The same boot_id means the reboot did not
   happen. Neither a login nor a new boot_id: exit 6 (after `reset`) or 10
   (after a started image): needs a person, report, do not retry.

Rebuild the transcript one last time from the whole of `serial_read`.

### 6. Record and judge

```
python3 -I driver/tools/uboot-t6/prompt-reads.py record <rec>/transcript.txt <rec-dir-of-this-run>   # reads session
python3 -I driver/tools/uboot-t6/prompt-go.py judge <rec>/transcript.txt ...                          # go run
python3 -I tools/cts/t6-uboot-handover-summary.py <reads-record> <t6_gpu-record> [<t6_cold-record>]
tools/cts/t6-redact-check.sh <rec>
tools/cts/t6-uboot-log.sh <rec> <YYYY-MM-DD> <name> <sha8>
```

`record` and `judge` write the record directory's files (`uart.log`,
`typed.log`, `answers.txt`, `image.log`, `verdict.txt`) and exit with `run`'s
code (below). The summary prints the measured hand-over block (exit 0; 2
usage; 3 an address in the input; 4 a record unreadable). `t6-redact-check.sh`
exits 0 clean, 1 a finding (file, line, kind; fix the copy by hand), 2 usage.
`t6-uboot-log.sh` copies the redacted record into
`tools/cts/log/<date>-t6-uboot-<name>-<sha8>/` and exits 0, or 1 when
something survived redaction, 2 usage or no text file, 3 destination exists.
Run the redact check on the copy that goes into the tree; nothing of
`records/` leaves unchecked. For `t6_fw`, `t6_submit` and the other probes,
`T6_ROUTE=uboot driver/tools/kexec-zephyr/t6-verdict.sh <probe> <rec>/image.log`
judges the image apart (0 PASS, 1 FAIL, 2 usage).

For dEQP rows, step 8 of the README's runbook (`t6-uboot-rows.sh`) opens the
line itself and cannot run through the lease: do one row at a time with
this file's steps 1 to 7, `<probe>` being the row's label, and only after
`gl_test` passed.

### 7. Undo

```
BOARD_HOST=<root@board> driver/tools/uboot-t6/stage.sh undo <fp>
```

Run it when the session ends, passed or stopped (and after an exit 4). It
removes `/boot/yakogl`; exit 0, or 1 when a stock file differs (named; the
directory is removed all the same), 2 usage, 3 refused. If the board is not
back in Linux, do not undo: report it as needing a person.

## Exit codes of a run

`prompt-reads.py record` and `prompt-go.py judge` answer with `run`'s codes,
each with one meaning:

| Code | Meaning |
|---|---|
| 0 | done; for a go run the image ran, the verdict is PASS and Linux's login came back |
| 2 | usage |
| 4 | the autoboot was missed: one space written, Linux booted (undo, retry once at most) |
| 5 | stopped at the prompt (an echo never matched after 3 tries, a command without an answer, the prompt lost): the board stays at the U-Boot prompt until its power is cut: needs a person |
| 6 | an answer differed or the reads ended, `reset` entered, no Linux login on the UART and no new boot_id: needs a person |
| 7 | U-Boot left the prompt during a command and started Linux |
| 8 | an answer differed (size, load, magic or CRC): `go` not entered, `reset` entered, Linux's login came back |
| 9 | `go` entered and U-Boot did not answer it (no `## Starting application`): nothing more written |
| 10 | the image started, its verdict was taken, no Linux login and no new boot_id: needs a person |
| 11 | Linux's login came back and the verdict is FAIL (`verdict.txt` names the lines) |
| 12 | no autoboot line in time: nothing was written |

The wrappers' own 1 (the board did not come back into Armbian), 3 (refused
before the reboot), 40 and 41 (the RFC 2217 session) have no meaning here:
there is no wrapper and the lease's server holds the line.

## A person is needed when

- the code is 5, 6 or 10 (or 9 with no sign of Linux), or Linux is not back
  within the waits of step 5;
- the lease tools fail, the transcript tool reports a gap or a decode error,
  or the echo check fails 3 times;
- anything on the board differs from what the tools print (a stock `/boot`
  file in `stage.sh`'s answer, an unexpected U-Boot answer).

Then: stop writing, report the record directory's name, the last transcript
lines and the code, leave the staging as it is, and do not retry.
