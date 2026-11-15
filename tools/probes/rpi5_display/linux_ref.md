<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# linux_ref.sh: the Pi 5 scanout state under stock Linux

`linux_ref.sh [OUTDIR]` runs **on the Raspberry Pi 5**, under its stock Linux,
before the Zephyr display probe. It records what the firmware set up for the
screen, as the reference the probe's readings are compared with (U3,
`driver/docs/rpi5-display.md`). It is strictly read-only.

Run it over ssh (`bash -s -- OUTDIR < linux_ref.sh`) or from a staged copy. As
root it reads more (debugfs, `/dev/mem`); as a normal user the unreadable parts
become `skip:` lines.

## Output

One `key: value` line per fact on stdout and in `OUTDIR/linux_ref.txt`
(OUTDIR defaults to a `mktemp -d` under `$TMPDIR`). The last line is
`linux_ref: done (N facts, M skipped)`. A part that cannot be read prints
`skip: <reason>`.

| Keys | Source |
|---|---|
| `get_throttled`, `vcgencmd_version`, `display_power` | `vcgencmd` (record `get_throttled` with every run) |
| `dt.<path>.status/compatible/reg/reg-names/width/height/stride/format` | `/proc/device-tree`: nodes named `hvs*`, `pixelvalve*`, `hdmi*`, `framebuffer*`, `vc4*`, `gpu`. `reg` is printed as raw 32-bit cells. |
| `drm.<connector>.status/enabled/dpms/modes` | `/sys/class/drm/card*-*` |
| `fb.<fbN>.name/virtual_size/bits_per_pixel/stride` | `/sys/class/graphics/fb*` |
| `debugfs.<card>.<file>[i]` | `/sys/kernel/debug/dri/<n>/` `hvs_*`, `*regs*`, `hdmi*`, `pixelvalve*` (first 64 lines each; includes `hvs_dlists`) |
| `iomem[i]` | `/proc/iomem` lines naming hvs, pixelvalve, hdmi, vc4, framebuffer, v3d |
| `modules` | `/proc/modules` filtered for display modules (read only) |
| `mem.hvs[off]`, `mem.pixelvalve0[off]`, `mem.hdmi0_hd[off]` | raw 32-bit words from `/dev/mem`, only when no debugfs `*regs*` dump was read |

`/dev/mem` ranges: HVS 0x10_7c580000 (64 words), pixel valve 0 0x10_7c410000
(16 words), HDMI0 `hd` block (16 words; its address comes from the `hdmi@`
node's `reg` and `reg-names`, the first `hdmi@` node in sorted order, so check
that it is HDMI0). Offsets are word offsets, not register names: map them with
`driver/docs/rpi5-v3d71-facts.md` section 10 and the Linux sources. The helper
is `python3 -I` with `mmap` `PROT_READ`, else `busybox devmem ADDR 32` with no
value; with neither the line is `skip:`. Nothing is installed.

## What it never does

No register or file writes, no `modprobe`/`rmmod`/`insmod`, no service
control, no touching of Tailscale, sshd, `config.txt`, `cmdline.txt`, the
EEPROM or the U-Boot environment. The only file written is `OUTDIR/linux_ref.txt`.

Redaction: every line goes through a filter that replaces IPv4/IPv6-like and
MAC-like strings and `serial...: value` pairs. Serial-number properties are
never read, and host names are never printed.

## Test

`bash tools/probes/rpi5_display/linux_ref_selftest.sh` runs the script against a
fake root (`ROOT=<dir>` prefixes `/proc`, `/sys` and `/dev/mem`; only the test
sets it, and with `ROOT` set `vcgencmd` is not called). It checks the
simple-framebuffer facts, the `hvs_dlists` lines, the `/dev/mem` fallback, the
redaction and the last line, plus `bash -n` and `shellcheck` when installed.
