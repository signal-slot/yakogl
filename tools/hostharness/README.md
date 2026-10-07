# Host harness: differential checker for libyakogl

The harness runs GL/EGL call streams through a library build on the host
(x86-64 Linux, no GPU) with a recording platform layer, and writes one log
per run that captures everything the library hands to the platform and
everything it hands back to the application. Two builds of the library
(C and Rust, or the port before and after a change) are compared by diffing
those logs.

Nothing is rendered: the fake GPU never executes a kick. That limits what a
log can show (see "Not compared"), but the command stream the library
produces is the thing that reaches the hardware, and the harness records it
completely.

**What a clean run means.** Against the previous Rust build the comparison is
the regression gate and every difference has to be explained. Against a C
tree it is not: the Rust port deliberately diverges from upstream C where the
C is wrong (README.md, "Status"), so such a run reports those divergences —
the default surface cleared (an extra `cache_flush` at context create), the
ES 3.0 / EGL 1.5 conformance fixes upstream has not taken, a vertex range
whose byte offset wraps 64 bits, and the runs where the C library aborts
under AddressSanitizer and the port does not. Since upstream `a444fe9` the
uniform blocks, the EGLImage lifetime, the padded pitch, the checked ranges
and `glIsVertexArrayOES` (gl_test case 22, which the driver's suite now
expects to answer per ES 3.0 6.1.10) agree on both sides. Recognise
those before looking for a regression.

## Files

| file | what it is |
|---|---|
| `rec.c`, `hh.h` | the recording OSAL: all 17 `struct osal_ops` entries (PowerVR kicks, or the V3D's with `-DYAKOGL_GPU_V3D`) |
| `gen_wrap.py`, `hh_wrap.h`, `wrap_rt.c` | generates and runs the entry-point interposer |
| `gltest.c` | program 1: the driver repository's `tools/pvrlab/gl_test.c` (every case) |
| `scen.c` | program 2: state / query / EGL / program-link / texture / buffer / FBO / stress / context / window sweeps |
| `hostile.c` | program 3: hostile sequences (wrapping range checks, misaligned pointers, EGLImage ownership, odd widths, size extremes, allocation failures) |
| `run.sh` | builds a library with sanitizers, builds the programs against it, runs every job in every mode |
| `report.py` | outcome table, A/B comparison, cross-mode checks, entry-point coverage |
| `compare.sh` | the whole differential check for two libraries (dev and release) |
| `allow.txt`, `allow-mesa-pds.txt` | documented differences the comparison accepts (`allow.txt` always; the others for one comparison each, named in their header) |
| `shim/` | the two Zephyr headers `gl_test.c` includes |

## Usage

```sh
W=/var/tmp/yakogl-work
# one library, all jobs, all modes (logs in out/logs/<job>.<mode>.{log,err,rc,calls,time})
tools/hostharness/run.sh all out rust:. $W/drv dev
python3 tools/hostharness/report.py status out

# two libraries: exit status 1 on an unexpected difference, report in out/report.txt
tools/hostharness/compare.sh out csrc:$W/oldsrc rust:. $W/drvold
# a build before source-form step 3 (main up to land123) vs one after it: the
# draws whose vertex shader reads gl_VertexID/gl_InstanceID use Mesa's layout
HH_ALLOW=tools/hostharness/allow-mesa-pds.txt tools/hostharness/compare.sh ...
# the port before and after a change
tools/hostharness/compare.sh out rust:/path/to/before rust:. $W/drv
```

Library specifications: `rust:<crate dir>` (either the workspace layout with
`capi/`, built as `-p yakogl-capi`, dev = profile `dev-lto`; or the older
single-crate layout), `csrc:<yakogl C tree>` (every C file compiled by
`run.sh`), `prebuilt:<libyakogl.a>:<tree for the headers>`. The C parts of a
Rust build (`src/osal/linux.c`, the driver's three generators, the layout
guards, `capi/eh_personality.c`) are compiled by `run.sh`; a layout guard
that does not match the driver stops the build.

Environment: `HH_SAN=0` (no sanitizers), `HH_MODES="l0 l2"`, `HH_SIZE=small`,
`HH_JOBS`, `HH_TIMEOUT`, `HH_SHADERS` (use the same shader tree on both sides),
`HH_PROFILES`, `HH_ALLOW`, `HH_SKIP_RUN=1`, `CARGO_NIGHTLY`, `HH_FEATURES`
(the cargo features of a `rust:` build; default the PowerVR's).

### The V3D (Raspberry Pi 4)

```sh
HH_FEATURES=osal-linux,gpu-v3d,shaderc-online tools/hostharness/run.sh all out-v3d rust:. driver dev
```

A `gpu-v3d` build compiles the V3D job builder (`driver/src/v3d/v3d_job.c`,
`v3d_tex.c`, `v3d_tiling_mesa.c`, `v3d_frame.c`) and its FFI guard instead of
the PowerVR generators, and runs `scen` and `hostile` only: `gl_test.c`'s host
arm is the PowerVR's (its V3D arm runs on the Zephyr image). `src/osal/linux.c`
is then the host model of the Pi 4's OSAL (its section "the V3D": one
carve-out, the board's allocator and free rule, `YAKOGL_V3D_*` settings), and
`rec.c` answers the V3D's way:

* its own allocator hands out 32-bit addresses (two ranges, data and shader,
  that never meet; an allocation past its range is refused, -12);
* `OSAL_QUERY_V3D_CPU_OF_GPU` from its records (the host copy, also under
  `HH_NOCPU`); `OSAL_QUERY_CLOCK_NS` unanswered (a time would differ between
  runs); the V3D's settings (VPM, draws a job, split depth/stencil, ...)
  from linux.c, which `rec.c` initialises in every mode for that;
* `submit` logs the `struct v3d_kick` fields; under `HH_BASE` the kick also
  goes to linux.c, which checks it as the board's OSAL does and gives back
  the frees it held for the job.

State (2026-10-07, dev, `HH_SAN=0`, all 62 jobs in all 12 modes): every job
ends `ok` in 10 modes. In `l0nc` and `l2nc` (object storage without a CPU
view) 21 jobs each panic, all with the same `ptr::write_bytes requires that
the destination pointer is aligned and non-null`: the V3D backend writes
through `mem.cpu` without checking it (one traced: the tiled copy's canary,
`src/v3d/backend.rs`, the allocation in the upload hook that runs inside
`glTexImage2D`, which `HH_NOCPU=1` counts as object storage).
`gles_osal.h` allows a NULL `cpu`, so this is a backend defect, but no V3D
OSAL hands one out (`zephyr_v3d.c` and `linux.c` always map), so it cannot
happen on the Pi 4. Not compared yet: no V3D run has a reference log, and the
`scen.progs-failat*` offsets were chosen for the PowerVR's start-up
allocations.

## Sanitizers and profiles

* Rust: `cargo +nightly` with `-Zsanitizer=address` for
  `x86_64-unknown-linux-gnu` (the crate is instrumented; the precompiled
  `core` is not). `run.sh` stops if no nightly is installed; `HH_SAN=0`
  builds without. Rust has no UBSan: the dev profile's overflow checks and
  debug assertions (e.g. the alignment precondition of
  `slice::from_raw_parts`) play that part and abort through the panic
  handler, whose message is in the log.
* C (the OSAL, the generators, a `csrc:` library, the harness itself):
  `-fsanitize=address,undefined` (UBSan reports are collected, not fatal).
* Every job runs in the dev and the release profile (`compare.sh`); release
  is the shipped configuration (overflow checks off). For a `csrc:` library
  dev is `-O1` and release `-O2`. A `prebuilt:` archive is whatever it was
  built with (the CMake host builds are not sanitized).

## What is recorded

`rec.c` implements the platform itself (its own allocator, fences and
registers); `src/osal/linux.c` is linked, and mode `base` routes
allocation, free, read and write through it.

| op | logged |
|---|---|
| `init`, `shutdown` | every call; repeated `init` keeps the state (gl_test.c re-initialises the platform per EGL case, which used to reset the fence counter so every `wait` failed) |
| `gpu_alloc` | heap, size, flags, result, device VA, record number; injected failures (`HH_FAIL_AT`, `HH_FAIL_EVERY` past `HH_FAIL_AFTER` (48), `hh_fail_alloc_in()`) |
| `gpu_free` | record, VA, size, **hash of the contents at the free**, whether kicks are in flight; freeing an unknown or already freed descriptor, or a changed one, is an `HH-ERROR` |
| `mmu_map` | VA as record + offset, physical address as application region or as the pages of another record; ranges that are not a whole live allocation are `HH-ERROR` |
| `map_phys` | absent (like linux.c), or a fake one with or without a CPU view (`HH_MAPPHYS=1/2`) |
| `gpu_write`, `gpu_read` | record, offset, length, hash of the data; out-of-range access is `HH-ERROR` |
| `cache_flush`, `cache_invd` | record + offset + length (or region); a range past the end of its allocation, on freed memory or on no allocation is `HH-ERROR` |
| `submit` | hash of exactly `kick_desc_len` bytes, every register field of `struct pvr_render_kick` decoded, in-flight count, then a memory dump |
| `wait`, `completed` | fence, timeout, result, completed/submitted |
| `reg_read`, `reg_write` | offset, value |
| `query` | id, size, result, hash of the output |
| `log` | every line, numbered, in the same stream (so the order against the other events is compared) |

Memory: at every submit every live allocation is hashed (the lines of those
that changed since the previous dump are printed, plus a digest over all),
again at every free, and all of them at program end, together with the
application regions (window buffers, native pixmaps: `hh_region_add()`).
Allocation records grow without limit (no silent drop); running out of host
memory aborts loudly. Fresh allocations are filled with `HH_FILL` (0x00 like
linux.c, 0xA5 like an uninitialised pool). `HH_NOCPU=1` hands out object
storage (allocations made inside `glBufferData`, `glTexImage2D`,
`glRenderbufferStorage`, `eglCreatePbufferSurface`, ...) without a CPU
mapping, so the GL core's `gpu_write`/`gpu_read` fallbacks run; the PowerVR
backend's own arenas and programs keep theirs (it refuses to start without,
see mode `ncall`). Physical addresses of allocations are
their host addresses (as in linux.c), so a window surface on a pool buffer
(gl_test case 21) maps real memory.

Fences: `HH_LAG=N` lets `completed()` trail `submit()` by N kicks; `wait(f)`
completes f. With lag the backend's in-flight paths run (frame-set reuse,
finish-previous, deferred frees). `HH_WAIT_FAIL=k` makes every k-th wait on a
pending fence time out (-110), for the backend's wait-failure paths.

Application side (`gen_wrap.py`): every exported `gl*`/`egl*` function the
programs call goes through a generated wrapper (the programs' objects are
rewritten with `objcopy --redefine-syms`, so the library's own internal
calls are not affected, unlike `-Wl,--wrap`). The wrapper counts the call and
logs, after the call: integer, enum and string arguments; the return value
(handles as small stable ids, strings verbatim, `glGetError`/`eglGetError`
only when not zero/`EGL_SUCCESS`); values written through output pointers
(`glGet*v`, `glGetUniform*`, `glGetVertexAttrib*`, `glGetTexParameter*`,
EGL queries, ...: the number of values written is measured by running the
call twice against sentinel buffers first, so an over-long write shows as a
longer list; the third call uses the application's own pointer); info logs,
sources and names; generated names; config lists; `glReadPixels` data (hash
of the bytes the call is specified to write); `glMapBufferRange` data for
read access; `eglGetProcAddress` (mapped to the wrapper the application
links against).

Modes (`run.sh`): `l0` `l2` (lag), `l0nc` `l2nc` (no CPU view), `l0mp`
`l0mpc` (map_phys without / with CPU view), `z0` `a0` `z2` `a2` (fill 0x00 /
0xA5 with normalised hashes), `base` (linux.c), `l3wf` (lag 3, every third
wait times out); opt-in: `ncall` (no allocation has a CPU view; the backend
refuses to start).

## What is compared

`report.py compare A B` diffs every `<job>.<mode>.log` pair. A difference is
unexpected unless every changed line pair is explained by `allow.txt`
(plus `--allow` files); outcomes (sanitizer report kind and access size,
panic location, exit status, timeout, `gl_test` return code, UBSan reports,
`HH-ERROR` count) must match too. gl_test runs that return non-zero are
listed separately even when both sides agree, with the first failing check,
so an early failure is never compared silently.

`report.py modes RUN` checks, inside one build:

* fill 0x00 vs 0xA5 (`z0`/`a0`, `z2`/`a2`): the whole log. Memory and data
  hashes treat 0x00 and 0xA5 alike, so a difference means the library
  derived something from memory it never wrote.
* CPU view vs none, lag 0 vs 2, map_phys variants, harness allocator vs
  linux.c: the application-visible lines only (wrapper results, gl_test's
  own output, `HH-ERROR`), which must not depend on the platform.

`report.py coverage RUN...` lists the exported entry points gl_test.c never
calls and those no program calls.

## Not compared / limits

* No rendering: render targets hold only what the CPU wrote, so read-backs
  compare what the library returns from that memory, and gl_test's pixel
  checks cannot pass (`expect_px` is patched to report only; checks through
  other helpers still fail, e.g. cases 18, 21 and 27-33: those return codes are
  listed, not hidden). Case 22 is different: it
  passes on both sides since the driver's `8213f55` and upstream's `4490ced`:
  a never-bound name is not an object (ES 3.0 6.1.10), which is what the case
  now expects.
* Float arguments are not logged (the calls are the programs' own inputs,
  identical on both sides); float results are logged as bit patterns.
* Pointer values (client arrays, mapped pointers) are not compared; mapped
  data and offsets are.
* The kick hash includes the struct's padding bytes.
* Fill comparison: a byte copied verbatim from unwritten memory hashes the
  same under both fills (normalisation) and is not detected; values
  derived from it are.
* `cache_flush`/`cache_invd` are recorded, not simulated: a missing flush
  shows as a missing log line, not as stale data.
* `base` mode does not see linux.c's internal border-colour page.
* The pack alignment used to size `glReadPixels` hashes is tracked from the
  programs' `glPixelStorei` calls, not per context.
* Sentinel-probed getters run three times; the extent probe scans 8 KiB.
* stderr (sanitizer reports) is summarised into the outcome, not diffed
  line by line (addresses and frames differ between builds).
* x86-64 only. Float-to-int conversions whose C behaviour is undefined give
  x86 results that aarch64 does not (see `allow.txt`); fp-contraction
  differences of an aarch64 gnu11 C build cannot show here at all.
