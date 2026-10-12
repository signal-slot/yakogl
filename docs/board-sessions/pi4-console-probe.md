# Pi 4 console bridge probe (2026-10-13)

Integration commit: 40dd2107fa2c

## Result: the bridge accepts a connection and carries the board's console

1. Inputs. BOARD_UART was not in the session environment or the lease; the
   bridge host:port came from the farm's notes (`ask_farm`): a raw TCP
   serial bridge on a USB serial node of the tailnet, one client at a time.
   The board's ssh login came from the lease.
2. Connect (python3 -I, socket, 5 s connect timeout): **accepted**. Not
   refused, no timeout, not closed by the peer, no 'busy' text. Reading for
   about 30 s with the board idle in Linux: **0 bytes** before the marker
   (the idle Linux console prints nothing by itself).
3. Marker over ssh. The board's kernel command line has
   `console=ttyAMA0,115200` (and `console=tty1`); `/dev/serial0` points to
   ttyAMA0. With the socket open, 10 s in, `echo PROBE_MARKER_77 >
   /dev/serial0` ran over ssh (exit 0). The marker arrived through the bridge
   as `PROBE_MARKER_77\r\n`, 17 bytes in total, nothing else. **The bridge
   carries the board's serial console.**
4. The socket was closed cleanly. The board was not rebooted; it is left in
   Linux, idle (load near 0, up about 24 min).

Note: an earlier probe of the same bridge (2026-10-11, branch of the closed
pull request) saw the connection accepted but the marker missing (2 stray
non-printing bytes). This run shows the marker arriving, so that failure was
not a permanent wiring/baud problem; it may have been transient (another
client, or the bridge's adapter state). Treat the console as working but
re-check on the run.

## What pi4-es2-tip-r2 should expect

- A TCP connection to the bridge works; the 'busy' handling should only
  trigger if another client holds it (one client at a time, so nothing else
  may be connected during a board.sh run).
- Idle Linux prints nothing on the console, so an empty capture before the
  firmware starts is normal; the marker test shows bytes written to the
  console UART are delivered unchanged with CRLF line endings.
- If console.txt is still empty or garbled during the run, record that as
  a blocker (bridge side) and do not guess results.
