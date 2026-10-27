<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Security policy

## Reporting a vulnerability

Do not open a public issue for a security problem.

Report it privately through GitHub's private vulnerability reporting: the
**Security** tab of github.com/signal-slot/yakogl, then **Report a
vulnerability**.

Please include:

- the affected commit, board (AM62P, i.MX8MP, Raspberry Pi 4) and GPU build,
- what you did and what happened, with a minimal test if you can,
- the impact you expect.

Do not attach the PowerVR firmware or any credentials.

The project commits to no response time. Reports are read and answered as
the maintainers can.

## Scope

YakoGL runs without an OS GPU driver. The library talks to the GPU
directly: it sets up memory, builds command streams, boots the firmware
(PowerVR) and handles interrupts itself. Nothing below it enforces memory
protection between a client and the GPU.

In scope:

- memory-safety or logic errors in the library that let a GL/EGL caller
  corrupt memory, hang the GPU, or run GPU work outside the buffers it was
  given;
- errors in the in-process shader compiler or the precompiled shader tables
  that cause the same;
- the build and release tooling (`tools/`, `driver/tools/`) leaking
  secrets, or writing board addresses or credentials into files;
- the PowerVR firmware or other proprietary files shipped by mistake.

Out of scope:

- an attacker who already controls the CPU, the kernel or the bootloader of
  the board, or who has physical access;
- a client that is deliberately given raw GPU access by design; there is no
  GPU isolation between mutually distrusting clients, and one GL client per
  board is the supported model;
- vulnerabilities in third parties (Zephyr, Slint, Mesa, TI's firmware and
  DDK, U-Boot): report them upstream. Tell us if YakoGL needs a change;
- rendering differences, conformance failures and performance problems
  (use public issues; the known defects are the open issues labelled `bug`);
- denial of service by a client that submits very heavy GL work on its own
  board.

## Supported versions

Only `main` is maintained until the first release.
