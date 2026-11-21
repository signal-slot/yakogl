<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# imx95_scmi

**Question.** What does the System Manager (SM) tell the A55's non-secure
agent, read only: which agent our image is, which protocols at which
versions, and the state and rights of the GPU's and the display's power
domains, performance domains and clocks? The pre-study's B2 SCMI half and
U1; the display note's Z0 and U2. Nothing is set: no SET message is sent.

**What it does.** `yd_imx95_scmi_survey` (`soc::imx95::survey` in the
drivers crate) over the board's raw SCMI channel (`verdin_imx95_scmi`): the
base protocol (version, attributes, vendor, sub-vendor, implementation
version, the calling agent, the protocol list), every listed protocol's
version and attributes, then

- power domains 14 (GPU), 13 (DISPLAY), 3 (CAMERA): attributes and name,
  state;
- performance domains 9 (GPU), 12 (DISP), 11 (CAM): attributes and name,
  levels (index:kHz), limits, the current level;
- clocks 174, 84, 83 (the GPU's gate, root, APB), 75, 76, 78, 80 (DISPAPB,
  DISPAXI, DISPOCRAM, DISP1PIX), 18, 19 (VIDEOPLL1), 37, 38 (LDB PLL), 62
  (CAMAPB), 66 to 69 (the MIPI PHY's), 155 (TPM6), 48, 129 (LPI2C2, LPI2C3):
  attributes and name, rate, parent, permissions, configuration (enabled).

Only messages `survey::read_only` admits are sent; the host test
(`soc/imx95/survey_tests.rs`) runs the survey over the mock SM and checks its
log holds no write.

**Expected lines** (one per fact, the key stable):

```
imx95_scmi: wdog found CS ... timeout 60000 ms
imx95_scmi: base version = 0x00020001
imx95_scmi: base agent = 2 AP-NS
imx95_scmi: base protocols = 0x11 0x12 0x13 0x14 ...
imx95_scmi: proto 0x11 version = 0x00030001
imx95_scmi: pd 13 state = 0x40000000 (off)
imx95_scmi: perf 12 level = 3
imx95_scmi: clk 80 permissions = 0xe0000000 (state parent rate)
imx95_scmi: clk 75 permissions = 0x00000000 (none)
...
imx95_scmi: PASS 210 facts, 12 answered with a status, 0 broken
```

A message the SM refuses is a line with the status as its value
(`clk 174 parent = NOT_SUPPORTED (-1)`): a fact, not a failure.

**Reading it.** Save the console and run

```sh
tools/probes/verdin_imx95/imx95_scmi/check.sh console.txt
```

It compares the lines with `expected.txt` (Toradex's public SM configuration
and NXP's SM sources as the design notes cite them: agent 2 "AP-NS", the
protocol versions the port's sequences check, the GPU's and display's
levels, AP-NS's clock permissions) and prints one line per expectation that
does not hold, then `imx95_scmi check: N expected, M match, D differ, K
missing; verdict V`; exit 0 only when all match and the verdict is PASS.

- `base agent` other than `2 AP-NS`, or permissions other than expected:
  the board runs another SM build or configuration (pre-study U1); the GPU's
  and display's sequences may be refused (DENIED). Record the lines in the
  board's issue before running `imx95_ident`.
- `FAIL rc -5: the base protocol did not answer`: the channel is dead (MU2,
  the shared memory at 0x445b1000; the `rc` is the transport's errno); no
  later probe can run.
- `broken` not 0: a reply of the wrong form; its line says which.
- Keys not in `expected.txt` (the domains' states, the rates) are the record
  of what U-Boot left; they are not checked.

**Build**: `west build -p always -b verdin_imx95/mimx9596/a55 -DYAKOGL_WERROR=ON -d build/imx95_scmi yakogl/tools/probes/verdin_imx95/imx95_scmi`.
`check_selftest.sh` runs `check.sh` on canned consoles (`testdata/`).
