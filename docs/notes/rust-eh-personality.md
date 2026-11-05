<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# `rust_eh_personality` across rustc nightlies, and where yakogl resolves it

A host link of a yakogl Rust staticlib needs a definition of the Rust EH
personality routine, and since the rustc 1.101 nightlies the name to define
is not `rust_eh_personality` any more. This note says why the reference
exists, what changed, the three places that resolve it, and the rules for a
new host link. Checked on rustc 1.101.0-nightly (5d8fec066 2026-10-16), the
nightly this tree was built with. On rustc 1.98.0-nightly (f46ec5218 2026-06-30) the same host build names the plain `rust_eh_personality` (18 undefined references).

## 1. Why a `panic = "abort"` staticlib still names it

The workspace `Cargo.toml` sets `panic = "abort"` in `[profile.dev]` (and in
the release profile), and `driver/rust` (crate `yakogl-drivers`, staticlib)
has its own `#[panic_handler]` (`driver/rust/src/lib.rs`). Nothing of ours
unwinds. For a hosted target the staticlib still carries the precompiled
`core` and `compiler_builtins` of the sysroot, built with unwind tables, and
those objects reference the personality routine.

Checked with `cargo build -p yakogl-drivers --features pvr` (host,
x86_64-unknown-linux-gnu) and `nm -A -u` on `libyakogl_drivers.a`: the
undefined reference is in the `core-*.rcgu.o` member and in 13
`compiler_builtins-*.rcgu.o` members, in none of the crate's own objects.

On the board's target, `aarch64-unknown-none-softfloat`
(`driver/zephyr/CMakeLists.txt`), the same build has no personality
reference (`nm -u` on its `libyakogl_drivers.a`: none). The precompiled
libraries of a bare-metal target carry no such tables.

## 2. What the 1.101 nightlies changed

From rustc 1.101.0-nightly (5d8fec066 2026-10-16), the reference is rustc's
internal `__rustc::rust_eh_personality`, v0-mangled, for example
`_RNvCs..._7___rustc19rust_eh_personality`. The `Cs<hash>_` part changes with
every compiler build. A C file that defines plain `rust_eh_personality` no
longer satisfies the link:

    undefined reference to `__rustc::rust_eh_personality'

(ld demangles the v0 name.) Older nightlies and stable name the plain
`rust_eh_personality`, so any fix has to take both.

## 3. The three places that resolve it

None of them spells a mangled name. Each reads the undefined symbol off the
archive (or the toolchain's `compiler_builtins` rlib) with `nm`.

- **The C API archive**: `capi/assemble.cmake` reads the personality
  references from the Rust archive (`_personality_refs`, `nm -A -u`, either
  `rust_eh_personality` or an `_R...rust_eh_personality`) and renames them
  with `objcopy --redefine-sym` to `yakogl_rust_eh_personality`.
  `capi/eh_personality.c` defines that name and forwards to
  `__gcc_personality_v0`; the rename keeps it from colliding with the
  personality of a Rust std staticlib in the same program. For an external
  linkage build (`libyakogl_c.a`, the `YAKOGL_EH_PERSONALITY` option in
  `CMakeLists.txt`) the forwarder is compiled with
  `-DYAKOGL_EH_PERSONALITY=<name>`, the name read with `nm -u` from the
  sysroot's `libcompiler_builtins-*.rlib` (plain name if no rlib is found).
  Commit 1d146219 (2026-11-03, "capi: rename and define the eh personality
  under the name the archive uses").
- **The host harnesses**: `tools/hostharness/run.sh` does not post-process the
  archive. It reads the name off `libyakogl.a` with `nm -u` (the awk match
  accepts `rust_eh_personality` or `_R...rust_eh_personality`, and dies if
  there are several), and compiles `capi/eh_personality.c` with
  `-DYAKOGL_EH_PERSONALITY=<that name>`.
- **The pvr check rows**: in `tools/check.sh`, `pvr_eh_personality_alias`
  (about line 1915) reads the `_R...rust_eh_personality` symbol off the
  `libyakogl_drivers.a` with `nm -u` and prints
  `-Wl,--defsym=<it>=rust_eh_personality`. The `rgxlayer_check` and
  `km_check` rows add it to their `cc` line; the C harnesses
  (`driver/compat/pvr_rgxlayer_check.c` side, `tools/pvr/km/km_check.c`)
  define the plain name. Commit d9e34741 (2026-11-02, "check.sh: pvr
  rgxlayer_check and km_check link core's mangled personality"), then 2e949ed5
  (2026-11-04, "pvr check: the harnesses keep the plain rust_eh_personality
  that check.sh aliases the mangled name to"). Nothing is printed on a
  nightly that uses the plain name, so those link as before.

## 4. Rules

- No `rust-toolchain` pin to dodge the change: the gate follows the host's
  nightly.
- No hard-coded `_R...rust_eh_personality` anywhere in the tree (the hash is
  per compiler). `git grep -nE '_R[A-Za-z0-9_]*rust_eh_personality'` must stay
  empty; patterns that match a symbol read from `nm` are fine.
- A new host link of a Rust staticlib (a check row, a harness, a CMake
  target) must use one of the three mechanisms above: rename and forward
  (`capi/assemble.cmake`), define under the name read off the archive
  (`capi/eh_personality.c` with `YAKOGL_EH_PERSONALITY`), or `--defsym`
  (`pvr_eh_personality_alias`). It must work on both the plain and the
  mangled name.
- A forwarder has to be a real personality (`__gcc_personality_v0`), not an
  empty function: a foreign unwind through a Rust frame calls it, and the
  abort pad then stops the image (see the header of `capi/eh_personality.c`).
- The board build stays as it is: `aarch64-unknown-none-softfloat` references
  no personality, and none of this applies to it.
