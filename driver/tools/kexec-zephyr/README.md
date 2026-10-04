# kexec_zephyr

Boots a Zephyr AArch64 image on the Verdin AM62P straight from Torizon Linux,
using the `kexec_load` system call (the board's kernel has `CONFIG_KEXEC=y` but
no kexec-tools). No serial console or U-Boot access is needed: the image is
placed at its link address and the kernel jumps to it with the MMU off.

```sh
# host: freestanding, so any aarch64 GCC works (no libc or sysroot needed)
aarch64-linux-gcc -O2 -static -nostdlib -ffreestanding -fno-stack-protector -o kexec_zephyr tools/kexec-zephyr/kexec_zephyr.c
scp kexec_zephyr build/zephyr/zephyr.bin torizon@<board>:/tmp/
# board
sudo rmmod pvrsrvkm          # the stock GPU driver must let go of the GPU
sudo /tmp/kexec_zephyr /tmp/zephyr.bin
```

`kexec_run.sh` does the copy, the `rmmod` and the jump in one go, and first
waits for the board's boot to settle (a jump during early boot sometimes gave
a Zephyr that never reached its console):

```sh
BOARD_HOST=torizon@<board> tools/kexec-zephyr/kexec_run.sh build/zephyr/zephyr.bin
```

The image is `zephyr.bin` from a build with `CONFIG_AARCH64_IMAGE_HEADER=y`
(the SafeUI `verdin_am62p/am62p54/a53` board sets it); the default load
address 0x82000000 is that board's `zephyr,sram` base.

Linux comes back when Zephyr resets the SoC (`sys_reboot`, or the RTI
watchdog): U-Boot runs its normal `bootcmd`. DDR does NOT survive that reset
(see below), so a Zephyr log or result has to leave the board another way
(the UART, or the panel for a camera).

## The loader refuses to jump without a running watchdog (AM62P)

A Zephyr image entered this way has exactly one way back: the RTI watchdog
Linux left armed. The SafeUI SoC code feeds it for a bounded time
(`SOC_AM62PX_A53_FEED_INHERITED_WDT_SECONDS`) and then lets it reset the SoC --
but only if it is already running. If it is not, the feeder logs "RTI watchdog
not running, nothing to feed" and returns, and an image that hangs holds the
board until someone cuts the power. That happened on 2026-09-23: a Qt Quick 3D
demo was jumped to about 20 s into Linux's boot, hung, and its picture was
still on the panel hours later.

So `kexec_zephyr` reads MAIN RTI0's DWDCTRL (0x0e000090) through `/dev/mem` on
an AM62P (`ti,am62p5` in the device tree) and refuses the jump unless it reads
0xa98559da ("enabled"), exit status 3. `--no-watchdog` (or
`BOARD_NO_WATCHDOG=1` for `kexec_run.sh`) skips the check for a deliberate
experiment. On other SoCs it checks nothing and says so. `kexec_run.sh` also
waits for `systemctl is-system-running --wait` first; whether Torizon has
always armed the watchdog by then is not measured yet (the check makes that
question safe to get wrong). A runner that calls the loader itself should do
the same. The durable fix is for the image to arm the RTI itself when it finds it
stopped (the SafeUI SoC code, not this repository).

## What a previous Linux leaves behind (Verdin AM62P, Torizon 7.6.1)

- The GICv3 CPU interface stays in split EOI/deactivate mode; a Zephyr image
  must clear `ICC_CTLR_EL1.EOImode` before it enables interrupts (the SafeUI
  A53 SoC hook does), otherwise each interrupt is delivered once and the
  kernel hangs in its first timed wait.
- The K3 RTI watchdog (60 s, 50 % window) keeps running and cannot be
  stopped: service it in the open window (down counter below half the
  preload) or the SoC resets 30..60 s after the jump. The watchdog is also
  the only way back: nothing can reset the board remotely, so an image that
  halts one CPU on a fatal error while a timer on another CPU keeps feeding
  the watchdog hangs the board until someone cuts the power. Give the feeder
  a bounded lifetime (`SOC_AM62PX_A53_FEED_INHERITED_WDT_SECONDS`) and stop
  it from the fatal-error handler.
- `tidss` has a shutdown callback, so the panel is off when Zephyr starts;
  runtime PM may have powered I2C1 down. Both are handled by the display
  bring-up in `samples/zephyr/gl_triangle`.
- DDR does not keep its contents across the warm reset, so a RAM console or
  a result buffer cannot be read back from Linux afterwards; use the UART.

## Bus masters left running by Linux

The kexec path does not stop devices. The CPSW Ethernet receive DMA kept writing
frames into its old buffers after the jump, which by then were Zephyr's rodata
(2026-09-16, seen as text/rodata pages changing to `45 00 ..`/ARP/broadcast-MAC
words). The tool now takes the interface (`ethernet0`, `eth0` or `end0`) down
through rtnetlink right before the jump and ignores SIGHUP/SIGPIPE so it survives
the ssh session dropping with the link. Unbinding the driver instead oopses in
`free_netdev` on Torizon 7.6. Add other bus masters to `quiesce_bus_masters()`
as they are found.

## board_lock.sh: one board, several sessions

Every kexec runner used to open the shared lock and block on it:

```sh
exec 9>"$LOCK"
flock 9
```

That prevents two runs at once and nothing else, and a shared board produces
two failures it does not touch. Both were measured here on 2026-09-21.

**Waiting for ever.** Eight runs queued on the one lock, the oldest waiting 25
minutes. Each would have taken its turn long after the session that started it
had moved on, kexecing a board somebody else was by then using.

**Holding for ever.** One runner held the lock for 30 minutes and every other
session stopped. Its own wait loop had a 560 s cap, so the cap did not fire --
which is the point: a script cannot be relied on to bound itself at exactly the
moment it has gone wrong.

`board_lock.sh` bounds both. Source it in place of those two lines:

```sh
-exec 9>"$LOCK"
-flock 9
+. "$REPO/tools/kexec-zephyr/board_lock.sh"
```

The runner keeps its own `$LOCK`, which is taken as the default, so that is the
whole change. Sourcing acquires the lock; `LOCK_WAIT` (2700 s) gives up rather
than queueing, and once the lock is held a watchdog kills the run after
`HOLD_MAX` (900 s) however wedged it is. Set `BOARD_LOCK_NO_ACQUIRE=1` for a
script that only reads the board and wants the helpers.

Without `LOCK`, the lock is per board: `board-<12 hex digits of
sha256(BOARD_HOST)>.lock`, so runs on different boards do not queue behind each
other and the file name carries no address. Keep one spelling of `BOARD_HOST`
per board (an alias and an address would be two locks). Sourcing it with
neither `LOCK` nor `BOARD_HOST` set is refused.

The file lives in `/run/user/<uid>/yakogl-locks/`, the same path for every
session of the user on this host, whichever checkout or worktree it runs from.
That directory lasts as long as the user has any login session. If the
directory cannot be created, sourcing is refused; there is no fallback place.

### The watchdog must not hold the lock itself

`exec 9>"$BOARD_LOCK"` runs before the watchdog, so the watchdog subshell
inherits fd 9 and so does the `sleep` it forks. An flock is released only when
**every** duplicated descriptor is closed. The EXIT trap kills the subshell,
but `sleep` is its child: it survives, reparented, still holding fd 9, and
holds the board for the rest of `HOLD_MAX` after the run it was guarding has
finished. With `HOLD_MAX=1800` that is a half-hour dead hold after every run.

Hence `exec 9>&-` as the subshell's first act. Closing the watchdog's copy
does not release the parent's lock — one open file description, and the parent
still has its own fd — so a run holds the board exactly as long as it did
before; only the orphan loses the ability to. Killing the `sleep` in the trap
as well would also work, but `exec 9>&-` holds when the trap does not run at
all: a SIGKILLed run, or one that dies before the trap is installed.

**If you try to reproduce this, beware a clean result.** The first attempt
here came back with the lock free and no orphan — because the trap won the
race and killed the subshell *before* it had forked `sleep`. There was nothing
to orphan. In a real run the subshell has been alive for minutes and the
orphan is certain, so a short test reproduces the conditions under which the
bug cannot happen. Give the watchdog a second to fork before ending the run,
and check for the `sleep` by name:

```sh
lsof -t "$LOCK" | while read -r p; do ps -o pid=,args= -p "$p"; done
```

The same trap caught the reporting side: `board_lock_holder` used to filter
`sleep` out by name, so the report written to find holders was structurally
unable to see the holder it most often had. It does not filter it now.

**And do not try to identify an orphan from its parent pid.** Every
backgrounded job in these sessions is reparented to the harness's reaper, so a
parent of 1 or of the reaper is the NORMAL state for a healthy background
process, not evidence that anything died. A `sleep` holding the lock with no
live parent shell is equally likely to be:

* an orphaned watchdog, which should be killed; or
* somebody's deliberate reservation of the board -- `flock 9` then `sleep 900`
  to hold a window for an installer or a reboot. bash execs the final `sleep`
  into the same pid, so `ps` shows `sleep` with the *shell's* start time, and
  an elapsed time longer than the sleep's own duration is just the `flock`
  wait that preceded it.

Those two are indistinguishable by process inspection. One was killed here on
2026-09-22 in the belief it was the first when it was the second, taking out a
reservation someone had made to protect a user's install window. So:
`board_lock_holder` REPORTS; deciding that a holder is dead requires knowing
whose it was. Ask the owner, or wait for HOLD_MAX. It is a shared board and the
cost of waiting is far smaller than the cost of killing the wrong thing.

### Never edit a runner while one is running

bash reads a script incrementally from the file, by byte offset, as it
executes. Editing that file under a live instance shifts everything after the
point it has reached, and the running shell then parses the middle of a line as
though it were the start of one. The error it reports is at whatever line the
offset lands in, which has nothing to do with the edit and sends you looking in
the wrong place:

```
run_gc_slint_ring.sh: line 383: syntax error near unexpected token `('
run_gc_slint_ring.sh: line 383: `[ -n "$tj" ] || { echo "no jump -- rerun (...)"; exit 1; }'
```

That line is valid, and `bash -n` on the file passes. It was a run started 23
minutes before an unrelated edit to the same file, and it died AFTER kexecing
the board — so a board slot was spent and no reading came back, which on a
shared board is somebody else's turn as well as your own.

Take a snapshot and run that:

```sh
cp build/run_gc_slint_ring.sh build/run_gc_slint_ring.RUNNING.sh
timeout 3600 ./build/run_gc_slint_ring.RUNNING.sh --baseline
```

The copy cannot move under the running shell however much the original is
edited. Worth doing for any run long enough that you might improve the script
while waiting for it — which, for a kexec runner, is every run.

### Choosing HOLD_MAX

The default 900 s suits a runner that builds **before** taking the lock and
then only kexecs. A runner that builds while holding it needs more, and the
watchdog firing on a working run is worse than no watchdog, because it teaches
people to remove it.

The gl_test runners take the lock first: about 90 s of build, plus the sample's
own hold, which for a regression set at `DEADLINE=400` is `400 + 70 + 6*code`
and approaches 850 s with nothing wrong at all. Those set it higher before
sourcing:

```sh
HOLD_MAX=${HOLD_MAX:-1800}
. "$REPO/tools/kexec-zephyr/board_lock.sh"
```

Work it out from what your runner does *while holding the lock*, and leave
headroom for the slowest legitimate case. The bound is the point, not the
number. The same reasoning applies to any wait loop inside the lock: bound it
yourself with a message that says which wait failed, rather than leaving the
watchdog to kill the run with no explanation.

### Worktrees

Source from the main checkout (`$REPO`), not a worktree path (`$WT`), unless
your branch already contains this file. A worktree cut before `4131efc` does
not have it, and the runner will fail on the source line with nothing useful
to say.

It also defines `bssh` and `bscp`, which wrap the calls that actually hang:
`ConnectTimeout` covers a board that is already gone, and `timeout` covers one
that leaves mid-call, which `ConnectTimeout` does not.

When it gives up it names who has the lock open, oldest first. `ps` alone
cannot tell a holder from a waiter -- both show the runner -- because
`flock(1)` exits once it has the lock and leaves the shell holding the
descriptor. So a process with the lock open and no `flock` child is the holder;
one with a live `flock` child is still in the queue. `board_lock_holder` prints
that, and it is worth running by hand when the board seems stuck:

```sh
BOARD_HOST=<user@board> BOARD_LOCK_NO_ACQUIRE=1 . tools/kexec-zephyr/board_lock.sh
board_lock_holder
```
