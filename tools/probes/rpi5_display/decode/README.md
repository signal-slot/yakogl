<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_display/decode

The host side of U3 (`driver/docs/rpi5-display.md`, plan item P11 Display (U3)
in `docs/notes/pre-study-rpi5.md` §8): a Pi 5 scanout dump arrives as text - a
board record or the Linux-side `linux_ref.sh` report - and reading it by eye
means holding HVS word layouts in your head. `decode.c` reads such a dump on
stdin and prints what the reviewed headers say it means: each HVS6 channel's
state, its display list walked plane by plane, each pixel valve's setup and
timing, and the HDMI0 connector status. The decoding *is* the reviewed
header-only code - `hvs6_dlist.h` and `pv_hdmi.h` from
`driver/zephyr/lib/board/rpi5_scanout`, included unchanged - so the host reads
a dump with the same rules the image used to write it.

```sh
out=$(bash build.sh)       # build; prints the path it built (default $TMPDIR/decode; CC overrides the compiler)
"$out" <DUMP_FILE          # decode one dump; exit 0, or 1 when no register word was found
bash decode_selftest.sh    # build + hermetic self-test: "decode_selftest: N of N checks, 0 failed"
                           # (exit 77 with "gap: CC" when no C compiler is found)
```

Two dump shapes are accepted (the marker is searched for in a line, so a
console prefix like `rpi5_display: ` is tolerated):

| Line | From |
|---|---|
| `mem.hvs[0xOFF]: 0xVAL` (also `pixelvalve0`, `pixelvalve1`, `hdmi0_hd`) | `linux_ref.sh` (`../linux_ref.md`); its `dt.`/`fb.`/`skip:` facts carry no register words and are ignored |
| `u3 reg <hvs\|pv\|hdmi0> <name> +0xOFF=0xVAL` and `u3 dlist <ch> [<i>]=0xWORD` | the image's snapshot table and list capture (`driver/zephyr/lib/board/rpi5_scanout/snapshot.h`) |

What the decoder holds to, and why:

- The version word decides the channel group (C offsets from +0x30, D from
  +0x100). A version naming neither stepping decodes no channel - the same
  rule `src/main.c` states in its own `stopped not measured` line.
- `linux_ref.sh` reads 64 HVS words and 16 valve/hd words, so the display-list
  memory (+0x4000), the D-group channels and both VID_CTL words (+0x44,
  +0x48) normally are not in its dump; the decoder says `not in dump` rather
  than inventing them. A deeper `mem.*` read that carries the list anyway is
  walked from the word the channel's DL names.
- The image reads a channel's list twice (Z1, and again after its mailbox
  allocation), each read printing indexes from `[0]`: the capture resets at
  every `[0]`, so what decodes is the last read - the state after the
  allocation.
- A block the probe could not read answers all ones (unmapped valves; the hd
  region with the HSM clock answered 0 - the silent-stall rule at
  `rpi:vc4_hdmi.c:3251-3262`). All-ones words report as reading all ones and
  are not decoded: a read of ones is not a value.
- Words whose fields the headers do not name (HVS CONTROL, ctrl1, bgnd, cob,
  hdmi_dvp_ctl, the frame counters, `pv_inten`/`pv_intstat`/`pv_stat`/
  `pv_hact_act`) are counted but not printed: they are judged board-side, by
  `scanout_alive.h` and the record's own verdict lines.
- The output never carries an absolute bus address: register positions are
  byte offsets from their block's own base, list positions are word indexes,
  and plane buffer addresses print as offsets from the first plane's address
  in the same dump (`base+0x..`). The self-test checks that.

`testdata/` holds synthetic dumps in both shapes, assembled from the mock
vectors `driver/zephyr/lib/board/test/hvs6_dlist_mock.c` and
`pv_timing_mock.c`: a running 1920x1080 channel and an idle one (generation
C), a two-plane 1280x720 list walked from HVS memory, a generation-D table
with a unity XRGB8888 plane read twice, an idle HVS whose valves and hd region
were not read, and a version word naming neither stepping. They are not board
records; real records stay gitignored and redacted (`../README.md`). The
`dt.` fact line is written without its device-tree unit addresses: a real
report names the node `soc@107c000000/hvs@7c580000`, and a literal
`name@<unit>.status` line reads as an e-mail address to
`tools/release/redaction-scan.py` - the same trap `../linux_ref_selftest.sh`
avoids by building such names from an `AT=@` variable.

The gate does not run this self-test yet - no `tools/check.sh` row covers
`tools/probes/rpi5_display/` host tests (see `../README.md`); running it
locally is the check until a row exists, which the report of this change
names as a follow-up issue. The `licence` audit row does read these files:
every file here carries its SPDX header lines.
