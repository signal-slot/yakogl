<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Security policy

## Reporting a vulnerability

Do not open a public issue for a security problem.

Report it privately through GitHub's private vulnerability reporting: the
**Security** tab of github.com/signal-slot/yakogl, then **Report a
vulnerability**.

Please include:

- the affected `main` commit, the board (AM62P, i.MX8MP, Raspberry Pi 4) and
  the GPU build (the `gpu-*` Cargo feature),
- what you did and what happened, with a minimal test if you can,
- the impact you expect.

Do not attach the PowerVR or Mali firmware, or any credentials.

The project commits to no response time. Reports are read and answered as
the maintainers can.

## Scope

YakoGL runs without an OS GPU driver. The library talks to the GPU
directly: it sets up memory, builds command streams, boots the GPU firmware
(PowerVR) and handles interrupts itself. Nothing below it enforces memory
protection between a client and the GPU.

Report here a problem in anything this repository holds:

- the library: the Rust crate in `src/` (GL and EGL entry points, the GPU
  back ends, the shader compiler and its precompiled shader tables, the OS
  abstraction) and its C ABI in `capi/`; the headers in `include/`;
- the GPU drivers and command generators in `driver/`, including the Zephyr
  drivers, the firmware boot and the test and benchmark images in
  `driver/samples/zephyr/`;
- the samples in `examples/`;
- the host tools in `tools/` and `driver/tools/`: a tool that leaks
  secrets, or writes board addresses or credentials into a file;
- the website sources in `website/`;
- the PowerVR firmware, the Mali firmware or any other proprietary file
  committed by mistake.

Examples of what is in scope: a memory-safety or logic error that lets a
GL/EGL caller corrupt memory, hang the GPU, or run GPU work outside the
buffers it was given; a shader compiler or precompiled shader table error
that does the same.

Out of scope:

- an attacker who already controls the CPU, the kernel or the bootloader of
  the board, or who has physical access;
- a client that is deliberately given raw GPU access by design: there is no
  GPU isolation between mutually distrusting clients, and one GL client per
  board is the supported model;
- the upstream code vendored unmodified under `third_party/` (Mesa, the
  TI/Imagination PowerVR DDK headers, TI's Zephyr DSI driver): report it to
  its upstream, then tell us if YakoGL needs a change;
- software fetched at build time or running next to YakoGL but not in this
  tree: Zephyr, Slint, FemtoVG, U-Boot, the Raspberry Pi boot firmware, and
  the GPU firmware the user supplies (TI's PowerVR `rgx.fw`, the Mali CSF
  firmware): report it to its upstream;
- rendering differences, conformance failures and performance problems
  (use public issues; the known defects are the open issues labelled `bug`);
- denial of service by a client that submits very heavy GL work on its own
  board.

## Supported versions

Only `main` is supported, until the first tagged release. `CHANGELOG.md`
has a `0.1.0` section (dated 2026-10-27) that describes the project at
that version; no version is tagged yet, so 0.1.0 is not a separately
supported version. Fixes land on `main`; there are no maintained release
branches.
