<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Slint sample (the cluster on a board)

The one Slint sample: it renders the cluster UI (`ui/cluster.slint`) through
YakoGL, shows the window, checks the first frame, then animates for
`CONFIG_YAKOGL_BOARD_ANIMATE_S` seconds. The no_std Slint trees it links are
described in `../nostd/README.md`.

## Build inputs (`slint-build:`)

The image links Rust that is built from inputs outside the repository: the
patched Slint tree (`SLINT_NOSTD_DIR`), the femtovg and glow forks
(`NOSTD_PATCH_DIR`), the crates.io crates they pull in, and the `rustc` of the
building machine. The first console line names them, filled at build time:

```
slint-build: slint <HEAD 8>+<tree 12> femtovg <ver>+<tree 12> glow <ver>+<tree 12> lock <12> rustc <release> (<commit> <date>)
```

| Field | What it is |
|-------|------------|
| `slint` | git HEAD of `SLINT_NOSTD_DIR` (`nogit` when it is no checkout), then `+` and a SHA-256 of its `internal/` and `api/rs/` sources (`.rs`, `.toml`, `.slint`, `.glsl`; no `target/`, no `tests/`), cut to 12 hex digits. The hash is what tells two patched trees apart. |
| `femtovg`, `glow` | the crate's `Cargo.toml` version, then the same hash over the fork's directory. |
| `lock` | the first 12 hex digits of `rust/Cargo.lock`, or `unpinned` when the image was built with `-DSLINT_CARGO_LOCKED=OFF`. |
| `rustc` | the first line of `rustc -vV` of the compiler cargo used. |

A `312b` and a `c592` picture of the Pi 4 (`docs/notes/slint-pi4-fingerprint.md`)
can now be told apart by this line. Compare two images with
`strings zephyr.elf | grep '^slint-build:'` (the `.bin` has it too).
The line is built into the image by `rust/build.rs` (the file
`build-inputs.cmake` writes at every build) and printed by `src/main.c` with
`printk`; it does not touch the rendering, the fingerprint or the probes.

### What is pinned

- **crates.io:** `rust/Cargo.lock` is committed and every build runs
  `cargo build --locked` against it. Before this, the sample's manifest is
  generated into the build directory with no lock, so every fresh build
  resolved the crates again and took whatever versions crates.io offered that
  day. A dependency that would resolve differently now stops the build.
  To refresh the lock on purpose: build once with `-DSLINT_CARGO_LOCKED=OFF`,
  copy `<build>/rust/Cargo.lock` over `rust/Cargo.lock`, and commit it with the
  reason. The path crates (Slint, femtovg, glow) are in the lock by name and
  version only, so the lock is the same on every machine.
- **Slint base:** `build-inputs.cmake` stops the build when `SLINT_NOSTD_DIR`
  has a git history and its HEAD is not the commit `../nostd/README.md` patches
  (`SLINT_BASE_REV` in `CMakeLists.txt`; keep the two in step). A tree
  without `.git` is accepted and reports `nogit`; its hash is then the only
  identity.
- **Not pinned by the build:** the patched file contents (reported by the hash,
  not compared with a stored value, because the trees change with the patches in
  `../nostd/`) and `rustc` (a nightly; reported only).

## Console lines

| Line | Meaning |
|------|---------|
| `slint-build: ...` | The build inputs (above), the first line of the run. |
| `slint: animation: N frames in T ms` | The whole animation, once at the end. |
| `slint-frame: frames=N ms_per_frame=X.XX` | The frame rate, as ms per frame (below). |

### `slint-frame:` (the frame rate)

The animation loop prints one line per 120 frames, and a last line for the
frames left over when the animation ends (`frames` is then less than 120).
`ms_per_frame` is the `k_uptime_get()` time of the window divided by its
frames, rounded to 0.01 ms. A frame counts the whole loop pass: the Slint
draw, `eglSwapBuffers` and `show()`. The first frame (the one the probes
check) is outside every window.

Use the line as the Slint speed: it is the same-board cost of one frame, not
a demo fps. The line is printed with `printk` only; it does not touch the
rendered picture, the fingerprint record or the probes record.
A later tally item should parse it (`slint-frame: frames=(\d+) ms_per_frame=([\d.]+)`)
and weight the windows by `frames`.
