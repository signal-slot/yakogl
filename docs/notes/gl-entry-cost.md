<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The cost of glmark2's cheap GL state calls: the profile shim or the library?

`tools/hostharness/glmark2/FINDINGS-gc-allscenes.md` shows
`desktop:windows=4:effect=shadow` spending 0.60 ms a frame of GL time over
about 360 calls on the vivante host build: `glUseProgram` 0.077 ms (52
calls), `glEnableVertexAttribArray` 0.077 ms (52), `glVertexAttribPointer`
and `glBindBuffer` about 1.5 us a call. It says that is "mostly the
profile's per-call cost". This note measures how much of each cheap entry
point is the shim's clock reads (`tools/hostharness/glmark2/shim.c`) and
how much is the library, and what a library fix would gain.

## Result in short

* **The library's state setters are 4 to 10 ns a call** (hot loop, no
  shim): `glUseProgram` 5.6, `glBindBuffer` 9.8, `glEnableVertexAttribArray`
  3.7, `glVertexAttribPointer` 5.9. Only `glUniformMatrix4fv` is dearer, 42
  to 47 ns. The same on the vivante, V3D and PowerVR builds (they share the
  GL core).
* **The shim adds 13.4 ns a call** (a null call through its bracket): 45 to
  70% of what the profile reads for a state setter in the scene (19 to 31
  ns), 22% of `glUniformMatrix4fv`'s.
* **The 1.5 us a call in `FINDINGS-gc-allscenes.md` is neither the library
  nor the shim: it is the host.** On a quieter host the same scene reads
  0.047 ms a frame of GL time on the vivante build, not 0.60 (13 times
  less), and `glUseProgram` 30 ns, not 1.5 us. The earlier profile ran on a
  shared, loaded host (its own text says so). Take no absolute number from a
  loaded-host profile; "mostly the profile's per-call cost" in that file
  should read "mostly the loaded host".
* **Nothing in the cheap state setters is worth a library change.** Taking
  all four to zero saves under 2 us a frame in `desktop` shadow. The one
  setter with something to remove is `glUniformMatrix4fv` (about 25 ns a
  call, 1 us a frame in `ideas`).

## Method and commit

* yakogl: main `41e951696810` (`hostharness: compare.py ...`), unchanged
  while this ran; glmark2 `22c527cb0556f3a1ac4445aaa52cc532760928d5`, a
  checkout without local changes.
* Host: x86-64 Linux, 16 threads, load average about 5 on arrival (other
  jobs share it); the runs went one at a time, and the microbenchmark ran
  while no profile run did.
* (b) the profile, 6 repeats, the raw `gm2: gl-time` lines (ns, not the
  3-decimal ms of `profile-<gpu>.md`), minimum over the repeats:

```sh
for i in 1 2 3 4 5 6; do
  GM2_WRITE=0 GM2_PROFILE=1 GM2_FRAMES=60 GM2_ENTRIES='^desktop:' \
    tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/glmark2-entry vivante powervr v3d
  # keep build/glmark2-entry/logs/<gpu>/*.log of each repeat
done
```

  59 counted frames a run. Per call: `frames=<calls>,<wall ns>,<cpu ns>`
  divided. `GM2_PERF=1` was not run.
* (a) and (c): the microbenchmark at the end, uncommitted, linked against
  each `build/glmark2-entry/lib-<gpu>/libyakogl.a`; 5 runs of 7 x 200000
  calls, minimum. "direct" is the call in a loop; "bracketed" is the same
  call inside the shim's clock reads (`shim.c:454-501`, minus `gpu_now()`),
  taken as the shim takes it: the last read of `tick_begin` to the first of
  `tick_end`. The null call is an empty function through a pointer.

## Table: ns a call

The profile column is the minimum of 6 repeats in
`desktop:...effect=shadow` (52 calls a frame; 59 frames), vivante / v3d /
powervr. Microbenchmark columns agree on the three builds within the noise.

| entry point | profile in the scene | direct (library, hot) | bracketed (hot) | shim's share of the profile reading | spread of 5 runs, direct |
|---|---|---|---|---|---|
| null call | - | 1.0 | 13.4 to 14.2 | the floor, 13.4 | - |
| `glUseProgram` (2 programs alternately) | 29.7 / 31.5 / 31.1 | 5.6 | 18.4 to 18.9 | 13.4 of 30 = **45%** | 5.6 to 5.9 |
| `glBindBuffer` (2 buffers alternately) | not called in `desktop` | 9.8 | 22.8 to 23.4 | 13.4 of 23 = **58%** (hot) | 9.8 to 10.2 |
| `glEnableVertexAttribArray` | 19.2 / 20.9 / 19.3 | 3.7 | 15.9 | 13.4 of 19 = **70%** | 3.7 |
| `glDisableVertexAttribArray` | 21.1 / 21.2 / 21.6 | as Enable | | 13.4 of 21 = 64% | |
| `glVertexAttribPointer` | 23.2 / 25.0 / 22.6 | 5.9 | 17.6 to 17.9 | 13.4 of 23 = **58%** | 5.9 (one powervr run 10.2) |
| `glUniformMatrix4fv` | not called in `desktop` | 42.1 (v3d), 46.8 (vivante, powervr) | 57.2 to 71.2 | 13.4 of 60 = **22%** | 42 to 66 |
| `glBindTexture`, for scale | 33.9 / 35.7 / 35.1 | | | 38% | |

For `glUseProgram` the scene's 30 ns holds 13 ns of shim, 6 ns of library
hot, and about 11 ns more that only the scene shows (code and state cold in
L1 and the predictor after the scene's other calls). That remainder belongs
to the library and the shim together and this method cannot split it. The
thread-CPU column of the profile reads about 200 ns on every entry point:
it is the system call (README, Profile) and says nothing of the call; rank
by wall.

Per frame in `desktop` shadow (52 calls each of Use, Enable, Disable,
Pointer): about 4.8 us, of which the shim's floor 2.8 us and the library hot
1.0 us. The scene's GL time is 0.047 / 0.107 / 0.086 ms a frame (minimum of
6; maximum 0.090 / 0.149 / 0.149), the frame 0.181 / 0.247 / 0.225 ms.
`desktop` blur: GL 0.029 / 0.136 / 0.069 ms.

## Code path of each entry point

All at `41e951696810`. None calls the back end, takes a lock, allocates or
logs on the success path. Each begins with `gl_trace!` and
`context::current()`.

* **Common prologue.** `gl_trace!(name)` is `context::trace`
  (`src/gl/context.rs:1341-1349`): two relaxed stores, a load and add, one
  compare against `yakogl_trace_log_first`; about 1 ns. `context::current()`
  (`context.rs:1024-1031`) is `foreign_thread()` (`context.rs:1070-1073`: a
  relaxed load of `OWNER`, and when it is non-zero `is_foreign`,
  `context.rs:1079-1092`, which calls `osal::thread_id()`, so that call is
  made by every entry point once a context is current) and a load of
  `gl_ctx`. The 3.7 ns of `glEnableVertexAttribArray` bounds the whole
  prologue. No lock (the entry-point mutex of `src/osal/linux.c:1179-1183`
  is optional and the EGL side's).
* **`glUseProgram`**, `src/gl/program.rs:2050-2081`: `program_ref`
  (`context.rs:1539-1546`), the `linked` flag, `tf::capturing`, a store of
  `cur_program`, and a `delete_pending` read only when the program changed
  and the previous one was non-zero. The program takes effect at the next
  draw. 5.6 ns.
* **`glBindBuffer`**, `src/gl/buffer.rs:351-377`: two `binding_for_target`
  matches, `debug_bind_hist` (`buffer.rs:310-315`, always on: a ring store),
  `names::bind_slot` (`src/gl/names.rs:132-157`: a `holder()` scan of the
  far-name table for an in-pool name), the `used` and `target` stores and
  the binding store. 9.8 ns, the dearest of the four.
* **`glEnableVertexAttribArray`**, `src/gl/vertex.rs:141-149`: a bounds
  check and `c.attribs[index].enabled = true`. 3.7 ns.
  `glDisableVertexAttribArray` (`vertex.rs:151-160`) is the same.
* **`glVertexAttribPointer`**, `src/gl/vertex.rs:33-85`: argument checks, a
  `match` on `type_`, `client_array_with_vao`, seven stores. 5.9 ns.
* **`glUniformMatrix4fv`**, `src/gl/program.rs:2703-2705` into
  `set_uniform` (`:2490-2599`): `program_ref(cur_program)`, `uniform_entry`
  (`:2370-2424`) which rebuilds a `UniformEntry` from the location table on
  every call (`location_ref`, `declared_array`, `uniform_class`, a division
  `r.dwords / array_len`, two `element_off`), `setter_accepts`, a zeroed
  `tmp[16]`, a 16-element copy with a `transpose` test per element
  (`:2580-2585`), and two `store_dwords` (one per stage, `:2596-2597`).
  42 to 47 ns.

Error state: `set_error` (`context.rs:1572-1578`) runs only on an error; the
desktop scenes raise none (`gl_errors=0` in the scene line), so
`note_raised` (`context.rs:1173-1177`) is not on the timed path.

## What Mesa does for the same calls

From Mesa's source (main/shaderapi.c, bufferobj.c, varray.c, uniform_*.c),
not run here:

* `glUseProgram` (`_mesa_UseProgram`): context from TLS, a hash lookup of a
  new name under the shared-state mutex, link status,
  `FLUSH_VERTICES(_NEW_PROGRAM)`, reference counting.
* `glBindBuffer` (`bind_buffer_object`): an early return when the binding
  is unchanged, otherwise a name lookup and `_mesa_reference_buffer_object`
  (atomic increment and decrement).
* `glEnableVertexAttribArray` (`enable_vertex_array_attrib`): the VAO's
  enabled mask and dirty bits.
* `glUniformMatrix4fv` (`_mesa_uniform_matrix`): location validation, a
  `memcmp` against the stored value (an unchanged uniform sets no dirty
  flag), then the copy.
* With `mesa_glthread` the calls are marshalled into a batch; there is no
  equivalent here (one thread, by contract).

YakoGL's four state setters are at or below Mesa's cost; `glUniformMatrix4fv`
is level with it, without the unchanged-value test.

## Ranked fixes

Expected gain a frame is the avoidable ns times the calls a frame (raw logs:
`desktop` shadow 52 Use, Enable, Disable, Pointer and 26 BindTexture; blur
28 each; `ideas` 43 `glUniformMatrix4fv` and 40 `glVertexAttribPointer`,
`FINDINGS-gc-allscenes.md`). A board's A53 or A72 is 4 to 8 times slower
than this host.

| # | fix | files | gain a frame (host; board in brackets) |
|---|---|---|---|
| 1 | **The shim prints its floor.** At startup the shim times its own null bracket many times, prints `gm2: gl-floor <ns>`, and `report.py` writes it in the profile header with a warning when it is over 100 ns (a loaded or virtual host: do not use the absolute numbers). | `tools/hostharness/glmark2/shim.c`, `report.py`, README Profile paragraph | Changes no library time. Stops a 13x misreading like the one in `FINDINGS-gc-allscenes.md`; 13 ns x calls (2.8 us in `desktop` shadow) is the shim's part of every ms a frame. **The one fix worth an issue.** |
| 2 | `glUniformMatrix4fv` and the other `set_uniform` setters: build the `UniformEntry` once per location and link, return when the new values equal the stored block (Mesa's `memcmp`), skip the `tmp[16]` zeroing and the per-element transpose test. | `src/gl/program.rs` (`set_uniform`, `uniform_entry`) | About 25 ns a call (47 to about 20): `ideas` 43 calls = 1 us [5 to 8 us] of 1.6 ms. Marginal; an issue only if a board's reading of `glUniformMatrix4fv` proves much dearer than 8 x 47 ns. |
| 3 | `glBindBuffer`: return at the top when the target's binding already equals the name; `debug_bind_hist` out of release builds. | `src/gl/buffer.rs` (held by the running item `gl-buffer-retired-reuse`) | 5 ns a call, under 0.3 us a frame. Not worth an issue. |
| 4 | `trace` and `foreign_thread` (`osal::thread_id()` on every call): nothing to do. | `src/gl/context.rs` | 0 |
| 5 | `glUseProgram`, `glEnable/DisableVertexAttribArray`, `glVertexAttribPointer`: nothing to remove, 4 to 6 ns each. | `src/gl/program.rs`, `src/gl/vertex.rs` | 0 |

Fixes 2 to 5 together are worth about 1 us a frame on the host and under 10
us on a board, against 3 us for each of `ideas`' 162 `glDrawElements`
(`gl-draw-fixed-cost.md`): the cheap state setters are not where the
library's time is.

## What this does not show

* The in-scene remainder (about 11 ns of a state setter's 30) is cache and
  predictor state; only a board's own clock can say what it costs there.
* `glBindBuffer` and `glUniformMatrix4fv` are not in the `desktop` scenes,
  so their in-scene readings were not taken; the hot-loop figure is their
  lower bound. A run of `^ideas` and the `buffer:` entries adds them.
* The host was shared (load average about 5); the spread over the repeats
  is in the table. `GM2_PERF=1` was not run.

## The microbenchmark (uncommitted)

`gcc -O2 -I include micro.c build/glmark2-entry/lib-<gpu>/libyakogl.a -lm
-lpthread -ldl`; run it 5 times per library and take the minimum. It prints
`choose`, `surf` and `mc` lines to stderr (`mc 1` is a current context).

```c
/* micro.c: ns a call of the cheap state setters, with and without the shim's bracket */
#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <stdint.h>
#include <stdlib.h>
#define GL_GLEXT_PROTOTYPES
#include <EGL/egl.h>
#include <GLES2/gl2.h>

static uint64_t ns(clockid_t id) { struct timespec t; clock_gettime(id, &t); return t.tv_sec * 1000000000ull + t.tv_nsec; }
static void (*volatile nullfn)(void);
static void nothing(void) {}
/* the shim's bracket (shim.c:454-501), minus the gpu_now() calls */
static uint64_t acc_wall;
static inline void bracket_begin(uint64_t *w) { (void)ns(CLOCK_THREAD_CPUTIME_ID); *w = ns(CLOCK_MONOTONIC); }
static inline void bracket_end(uint64_t w) { uint64_t e = ns(CLOCK_MONOTONIC); (void)ns(CLOCK_THREAD_CPUTIME_ID); acc_wall += e - w; }

#define N 200000
#define REP 7
#define BENCH(label, stmt) do { double best = 1e18, bestw = 1e18; \
	for (int r = 0; r < REP; r++) { uint64_t t0 = ns(CLOCK_MONOTONIC); \
		for (int i = 0; i < N; i++) { stmt; } \
		double d = (double)(ns(CLOCK_MONOTONIC) - t0) / N; if (d < best) best = d; \
		acc_wall = 0; for (int i = 0; i < N; i++) { uint64_t w; bracket_begin(&w); stmt; bracket_end(w); } \
		d = (double)acc_wall / N; if (d < bestw) bestw = d; } \
	printf("%-28s direct %8.1f ns   bracketed wall %8.1f ns\n", label, best, bestw); } while (0)

int main(void)
{
	struct gles_native_window nw = { (uintptr_t)aligned_alloc(4096, (800 * 600 * 4 + 4095) & ~4095), 800, 600, 3200, GLES_NATIVE_FORMAT_XRGB8888 };
	EGLDisplay d = eglGetDisplay(EGL_DEFAULT_DISPLAY); eglInitialize(d, 0, 0); eglBindAPI(EGL_OPENGL_ES_API);
	const EGLint ca[] = { EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_NONE };
	const EGLint xa[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	EGLConfig cfg; EGLint n = 0; EGLBoolean ok = eglChooseConfig(d, ca, &cfg, 1, &n); fprintf(stderr, "choose %d n=%d err=0x%x\n", ok, n, eglGetError());
	EGLSurface s = eglCreateWindowSurface(d, cfg, &nw, NULL);
	EGLContext x = eglCreateContext(d, cfg, EGL_NO_CONTEXT, xa); fprintf(stderr, "surf %p ctx %p err=0x%x\n", s, x, eglGetError()); fprintf(stderr, "mc %d err=0x%x\n", eglMakeCurrent(d, s, s, x), eglGetError());
	const char *vs = "attribute vec4 p; uniform mat4 m; void main(){ gl_Position = m * p; }";
	const char *fs = "precision mediump float; void main(){ gl_FragColor = vec4(1.0); }";
	GLuint prog[2], buf[2], m[2];
	for (int k = 0; k < 2; k++) {
		GLuint v = glCreateShader(GL_VERTEX_SHADER), f = glCreateShader(GL_FRAGMENT_SHADER);
		glShaderSource(v, 1, &vs, 0); glCompileShader(v); glShaderSource(f, 1, &fs, 0); glCompileShader(f);
		prog[k] = glCreateProgram(); glAttachShader(prog[k], v); glAttachShader(prog[k], f);
		glBindAttribLocation(prog[k], 0, "p"); glLinkProgram(prog[k]); m[k] = glGetUniformLocation(prog[k], "m");
	}
	glGenBuffers(2, buf);
	for (int k = 0; k < 2; k++) { glBindBuffer(GL_ARRAY_BUFFER, buf[k]); glBufferData(GL_ARRAY_BUFFER, 4096, 0, GL_STATIC_DRAW); }
	float mat[16] = { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1 };
	int k = 0;
	nullfn = nothing;
	BENCH("null call", nullfn());
	BENCH("glUseProgram (alternate)", glUseProgram(prog[k ^= 1]));
	glUseProgram(prog[0]);
	BENCH("glBindBuffer (alternate)", glBindBuffer(GL_ARRAY_BUFFER, buf[k ^= 1]));
	BENCH("glEnableVertexAttribArray", glEnableVertexAttribArray(0));
	BENCH("glVertexAttribPointer", glVertexAttribPointer(0, 3, GL_FLOAT, 0, 24, (void *)(uintptr_t)(k ^= 12)));
	BENCH("glUniformMatrix4fv", glUniformMatrix4fv(m[0], 1, 0, mat));
	return glGetError() != 0;
}
```
