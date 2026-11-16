<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# decode: Pi 5 scanout register dumps, decoded on the host

`decode.c` reads a text dump on stdin and prints the HVS6 channels and
display-list planes, the pixel valve and the HDMI0 status, using the reviewed
headers `hvs6_dlist.h` and `pv_hdmi.h` (`driver/zephyr/lib/board/rpi5_scanout/`)
unchanged.

    bash tools/probes/rpi5_display/decode/build.sh OUT    # host cc, -std=c99 -Wall -Wextra -Werror
    OUT < linux_ref.txt
    bash tools/probes/rpi5_display/decode/decode_selftest.sh

Input, both sources (details in the comment at the top of `decode.c`):

- `linux_ref.sh` output: `mem.hvs[0xOFF]: 0xVAL`, `mem.pixelvalve0[..]`,
  `mem.hdmi0_hd[..]` (set `linux_ref`).
- the probe console log: `rpi5_display: D2 <when> <block>[0xOFF]: w w w w`
  (set `D2 <when>`), and the snapshot lines `u3 reg ...` and `u3 dlist ...`
  (set `u3`). Other lines are ignored. Each set is decoded on its own.

Output has no absolute bus address: registers are byte offsets from their own
block, list positions are word indexes, and a plane's buffer address is
`base+0x...` relative to the first plane of the set. Raw register words are
not printed, so the output can go into a redacted record.

Caveats from the dump sources, reported as "not in dump" rather than guessed:

- `linux_ref.sh` reads 64 HVS words, 16 pixel valve and 16 hd words: no display
  list (HVS +0x4000), no generation-D channels (+0x100), no `HD_VID_CTL`
  (+0x44, `pv_hdmi.h`).
- the probe's snapshot table names `hdmi_vid_ctl` at +0x0038, `pv_hdmi.h` says
  +0x44; the decoder prints the named line separately (unverified offset).
- a `u3 dlist` list is decoded from its index 0, as the probe's start word is
  not in the dump.

Exit 0 when any register word was found, 1 otherwise. Test dumps in
`testdata/` are built from the `hvs6_dlist_mock.c` and `pv_hdmi_mock.c`
vectors (1080p60, an unity XRGB8888 plane, two planes at 5120 pitch).
