<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# femtovg: one femtovg frame through YakoGL

[femtovg](https://github.com/femtovg/femtovg) is the 2D vector renderer
Slint draws with. This example draws one frame with it, through YakoGL's
OpenGL ES 2.0, into a 1024x600 window: a clear, a rounded rect, a rect with
a vertical gradient and a stroked polyline. Then it reads every pixel back
and judges it.

| Part | What it is |
|---|---|
| `src/main.c` | the window from the board layer, EGL, the program survey, the screen and the records |
| `src/programs.c` | femtovg's 30 programs built through GL before femtovg builds them, so a program that does not build is named |
| `rust/` | the staticlib: the scene drawn with femtovg over glow, the allocator (the C heap) and the C entry points |
| `check/` | the scene's constants and the check (crate `yakogl-femtovg-check`), host-tested by `tools/check.sh` |

## The check

Every pixel is put in one class by its position: inside the rounded rect,
inside the gradient, on the stroke, the background, the stroke's
anti-aliased fringe, the rounded rect's fringe, or the edges between them.
Each solid class must hold its colour (the gradient its mixed colour), each
fringe must be drawn partly, the edges are counted only. Then the guard rows
after the window must be untouched, and glReadPixels must agree with memory.
`check/src/lib.rs` has the classes, the margins and the tolerances.

The console ends with `femtovg: check PASS` or `femtovg: check FAIL: <what>`.
The records are the triangle's: PICTURE (the centre pixel and a CRC of the
centre column, from glReadPixels) after a checked frame, and an APP record
with the failure code after a failed check. The Pi 4 and the AM62P stored the
same PICTURE, `594b0407 ff201818 40e9c78c 04000258` (2026-10-25); the
i.MX8MP's is not measured yet.

## Build

The build needs the no_std femtovg and glow forks, which live outside this
repository (`../nostd/README.md` says how to make them; the Slint example
uses the same trees), a nightly `cargo` (`-Z build-std=core,alloc`) and the
target `aarch64-unknown-none`:

```sh
west build -p always -b rpi_4b yakogl/examples/femtovg -- -DNOSTD_PATCH_DIR=$WORK/nostd
```

`rust/Cargo.lock` pins what femtovg and glow pull in from crates.io; the
build uses it with `--locked`. To refresh it on purpose, build once with
`-DFEMTOVG_CARGO_LOCKED=OFF` and copy `<build>/rust/Cargo.lock` over it.
