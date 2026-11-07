<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# yakogl-mali-gles: libGLESv2.so.2 and libEGL.so.1 for the Mali over panthor

The GL pair dEQP loads on the NanoPC-T6 (Rockchip RK3588, Arm Mali-G610)
under its Armbian, through the panthor kernel driver. dEQP's `yakogl` target
(`tools/cts/cts-yakogl-target.patch`) opens `libEGL.so.1` and
`libGLESv2.so.2` at run time and looks up every entry point by name. The C
pair `CMakeLists.txt` builds (`-DYAKOGL_SHARED=ON`) compiles in a C OSAL and
reaches no GPU; this crate builds the same pair on the Rust development OSAL
the board's gl_test runs on (`yakogl-mali-panthor`,
`tools/mali/panthor/src/osal.rs`).

| file | what |
| --- | --- |
| `Cargo.toml`, `Cargo.lock` | the crate `yakogl-mali-gles`, a cdylib. Its own `[workspace]` table and lock file: the root workspace, its `Cargo.toml` and its `Cargo.lock` are not touched |
| `src/lib.rs` | the library: the `yakogl` crate's GL and EGL entry points, the load-time constructor, the two contract lines |
| `src/build.rs` | the `git describe` stamp of the loaded line |
| `src/deqp-entry-points.txt` | the 44 EGL and 142 ES 2.0 names dEQP looks up, from VK-GL-CTS 93bca01 (the file says where each list comes from) |
| `build.sh <outdir>` | builds both objects for `aarch64-unknown-linux-gnu` |
| `check.sh <outdir>` | host check of what `build.sh` wrote |

## What is in the library

- The `yakogl` crate (the repository root) with the backend features of
  `MALI_CONFIG` in `tools/check.sh` (`gpu-mali`, `shaderc-online`,
  `no-shader-table`), `host-test` replaced by the panthor OSAL, and
  `report-es2` (ES 2.0 reporting, as `tools/cts/host-deqp-run.sh` builds its
  gles2 pair; `build.sh --es3` leaves it out).
- `yakogl-mali-panthor`'s `osal::OPS`, the `OsalOps` table over panthor's
  uAPI: render node `/dev/dri/renderD128`, or `YAKOGL_MALI_DEV`.
- Neither crate is changed. The constructor (`DT_INIT_ARRAY`) installs
  bitwise copies of three tables with a few entries wrapped and points the
  library at them: the OSAL (`init`, `wait`, `log`), the backend (`draw`),
  the shader back end (`compile`, `link`, `compile_vs_linked`). Every other
  entry is the original.

libGLESv2.so.2 holds all the code and exports both the `gl*` and the `egl*`
entry points: EGL and GL share one process-wide state, so they are one
object (as in `CMakeLists.txt`). libEGL.so.1 is a forwarder: SONAME
`libEGL.so.1`, `DT_NEEDED libGLESv2.so.2`, `DT_RUNPATH $ORIGIN`. `dlsym` on
its handle searches the library it needs, and `$ORIGIN` finds the
libGLESv2.so.2 beside it, never the board's.

libGLESv2.so.2 also exports the `yakogl` crate's other `#[no_mangle]`
symbols (`gl_*` helpers, `osal_set_ops`, `g_osal`, `shc_*`, ...): rustc
exports every such symbol of a cdylib's crates and gives it its own version
script, so the C pair's `tools/yakogl.ver` cannot be applied. None collides
with a libc name; it is linked `-Bsymbolic`, so the library's own calls bind
to itself whatever else the process has loaded.

## The two lines (the runner's contract)

On stderr (dEQP's own output is on stdout), each in one `write`:

```text
yakogl-mali: loaded build=<git describe> lib=<path of this .so>
yakogl-mali-exit: draws=<n> refused=<n> first_refused=<name or -> gpu_faults=<n> job_timeouts=<n>
```

- `loaded`: once per process, at the first `eglInitialize` (the OSAL's
  `init`), before the device is opened. Only this pair's EGL reaches it, so
  it proves the run used YakoGL and not the board's glvnd + Mesa 25.0.7,
  which has the same two SONAMEs. `lib=` is the canonical path of
  libGLESv2.so.2.
- `exit`: at process exit (`atexit`). Absent when the process died (a
  signal, `_exit`), which is itself a finding.
  - `draws`: draws the backend recorded (its `draw` returned 0 with vertices
    to draw; clears are not counted).
  - `refused`: draws the GL core or the backend refused
    (`yakogl_draws_refused`) plus shaders and programs the compiler refused
    (`-ENOTSUP`, its "not lowered yet"; a real compile error is `-EINVAL`
    and is not counted).
  - `first_refused`: the first of those by name, in the gl_test sweep's
    words: `draw: <why>`, `compile: <log>`, `link: <log>`, whole (the
    backend's own string over the 255-byte log line, the compiler's log
    read into 8 KiB), each run of whitespace one space. `-` when none. It is
    the only field that may hold spaces: a reader takes everything between
    `first_refused=` and the last ` gpu_faults=`.
  - `gpu_faults`, `job_timeouts`: OSAL waits that found the work lost. A
    wait that timed out (rc 1) or found the group `TIMEDOUT` is a timeout;
    `-EIO` with the group `FATAL_FAULT`, or with no state to read, is a
    fault. A group found faulted or timed out at exit with no wait having
    seen it counts once.

## Build

```sh
tools/mali/panthor-gles/build.sh [--debug] [--es3] <outdir>
```

Needs cargo with the `aarch64-unknown-linux-gnu` target, clang, and for the
first build curl or python3, `ar` and `tar` with xz. The board's ABI floors
are GLIBC 2.41 and GLIBCXX 3.4.33 (Debian trixie), so the objects are linked
against a Debian trixie arm64 sysroot: four pinned packages (libc6,
libc6-dev, libgcc-s1, libgcc-14-dev; versions and sha256 in `build.sh`),
fetched from deb.debian.org (snapshot.debian.org once they leave the pool)
and unpacked once into `$YAKOGL_MALI_SYSROOT`, else
`$CONNECTED_DEPS_DIR/debian-trixie-arm64-sysroot` (the layout directory of
`tools/cts/deps-env.sh`). The linker is lld: `ld.lld` on `PATH`, else the
active Rust toolchain's `rust-lld`. `YAKOGL_MALI_CC=aarch64-linux-gnu-gcc`
links with a cross gcc instead (`check.sh` still checks the ceiling).

The cargo target directory is `target/` here (`CARGO_TARGET_DIR` moves it).
The panthor crate compiles gl_test.c only when it is given a cross compiler;
`build.sh` clears the variables it reads, so the library links no gl_test.

## Check

```sh
tools/mali/panthor-gles/check.sh <outdir>
cargo test --manifest-path tools/mali/panthor-gles/Cargo.toml
```

`check.sh` (host, no board): both objects are AArch64 shared objects with
their SONAMEs (`readelf -d`); libEGL.so.1 needs libGLESv2.so.2 with
`RUNPATH $ORIGIN`; libGLESv2.so.2 has its constructor; every name of
`src/deqp-entry-points.txt` is a defined function where dEQP looks for it;
no version need above `GLIBC_2.41` or `GLIBCXX_3.4.33` (`objdump -T` when
an objdump reads AArch64, else `readelf -V`); the contract lines' text is in
the object. Exit 0 when every check passed.

`cargo test` (host): the lines' formats, the refusal names, the wait
classification, the full-log compile wrapper, and the two lines end to end in
a child process (install, `eglGetDisplay`, `eglInitialize` twice with no
device: one loaded line, then the exit line).

## On the board

Copy both files into one directory and point the loader at it:

```sh
LD_LIBRARY_PATH=$dir ./deqp-gles2 --deqp-surface-type=pbuffer ...
```

and look for the `yakogl-mali: loaded ... lib=$dir/libGLESv2.so.2` line
before taking any result. `tools/cts/` holds the runner; this directory
holds no board script.
