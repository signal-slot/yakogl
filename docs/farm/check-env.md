# Check-environment setup for farm machines

`tools/check.sh` is the gate, and most of its rows need only ordinary
packages (rustup targets and distro packages). Until now only one farm
machine had all of them, so reviews queued for that one machine.
`tools/farm/setup-check-env.sh` lets any machine prepare the same
environment: it checks what is there, installs what it safely can, and
prints the rest as exact owner commands.

## What each tool unblocks

| Tool checked | check.sh row(s) it unblocks | Why it is needed |
|---|---|---|
| cargo >= 1.88 | every cargo row | `Cargo.toml` rust-version is 1.88 |
| rustup target `aarch64-unknown-none` | the three `check ... (aarch64-unknown-none, no host-test)` rows, `check osal-zephyr,gpu-powervr,precompiled-shaders (aarch64-unknown-none, no compiler)`, and `no Mesa reference in the artefact` | the product arms; without the target check.sh prints a gap and the gate is red |
| rustup target `aarch64-unknown-none-softfloat` | `check yakogl-drivers (aarch64-unknown-none-softfloat)` | the target the drivers ship for; a nightly with `rust-src` also covers this row, and the script accepts that |
| clippy | every `clippy` row, including `clippy yakogl-drivers, each shipped feature arm (-D warnings)` | the gate requires zero warnings |
| rustfmt | `cargo fmt --check` | formatting is a defect, not taste |
| clang, or a cross compiler named by `CC_aarch64_unknown_none` / `CC_aarch64-unknown-none` / `TARGET_CC` | the aarch64 check rows and `no Mesa reference in the artefact` | `build.rs` compiles the C wrappers (the csbgen wrappers, the v3d QPU files) for `os = none` with clang unless the environment names another compiler |
| libclang (`LIBCLANG_PATH`, or `find_library('clang')`) | `gen src/pvr/csbgen.rs + csbgen_wrap.c == bindgen` | bindgen loads libclang at run time |
| python3 + PyYAML | `gen driver/src/v3d/generated == Mesa's generators on third_party` | Mesa's `u_format_table.py` imports yaml |
| glslangValidator (`GLSLANG`) | `gen tests/reference/blobs.rs == tools/glsl2usc.sh on precompiled.list (pco_shc)` | glslang compiles the GLSL to SPIR-V for pco_shc |
| expat + zlib through pkg-config | `build v3d mesa reference (tools/v3d/mesa_ref.sh)` and the `run v3d ... (against Mesa 26.2.1)` rows | the Mesa reference links Mesa's CLIF decoder against `-lexpat -lz` |

Out of scope (per-machine workspaces and firmware, not packages):
`ZEPHYR_WS` with west and the Zephyr SDK (the three Zephyr image rows),
`HAL_NXP_DIR`, `PCO_SHC` with `GLSL2USC_CACHE`, `MESA_V3D_TREE`, and
`driver/src/fw/pvr_fw_blob.h`.

## Usage

```
tools/farm/setup-check-env.sh --check
```

Prints one `ok` or `missing` line per tool and exits non-zero if anything
is missing. Read-only.

```
tools/farm/setup-check-env.sh --install
```

- Always (user-space, idempotent): `rustup component add clippy rustfmt`
  and `rustup target add aarch64-unknown-none aarch64-unknown-none-softfloat`.
- Distro packages only when `sudo -n true` works. On apt systems it runs
  `sudo -n apt-get install -y clang libclang-dev python3-yaml glslang-tools
  pkg-config libexpat1-dev zlib1g-dev`. On Gentoo-like systems it prints
  the emerge equivalents instead of running them:
  `emerge -1 --noreplace sys-devel/clang sys-libs/clang dev-python/pyyaml
  virtual/pkgconfig dev-libs/expat sys-libs/zlib media-libs/glslang`.
- Without passwordless sudo it prints the exact owner command and looks
  for what is already on disk: a versioned or slotted clang
  (`/usr/lib/llvm-*/bin/clang`, `/usr/lib/llvm/*/bin/clang`, `clang-*`)
  and a directory holding `libclang.so*` for `LIBCLANG_PATH`.
- It prints, without applying it, the line
  `ccfarm env set yakogl CC_aarch64_unknown_none <compiler>` for the C
  compiler `build.rs` would use, so the farm can register the machine.

The script is idempotent and non-destructive: it only adds rustup
components, rustup targets and distro packages. It removes nothing,
overwrites no configuration, writes no file, and never edits an rc file —
every value it finds is printed for the owner or for `ccfarm env set`,
never exported into a shell startup file.

## Verify

1. `tools/farm/setup-check-env.sh --check` — all lines `ok`, exit 0.
2. `tools/check.sh --quick` — green: 0 failed, 0 gaps.
3. With the per-machine workspaces present, the full `tools/check.sh`.

Real `--check` output, from Lepton (Gentoo, 2026-10-08), exit 0:

```
ok      cargo                                  cargo 1.98.0-nightly (a335d47ff 2026-06-26)
ok      rustup-target-aarch64-unknown-none
ok      rustup-target-aarch64-unknown-none-softfloat
ok      clippy                                 clippy 0.1.98 (4c9d2bfe4a 2026-07-01)
ok      rustfmt                                rustfmt 1.9.0-nightly (4c9d2bfe4a 2026-07-01)
ok      cc                                     cc (Gentoo 16.2.1_p20260926 p1) 16.2.1 20260926
ok      clang                                  clang version 23.1.2
ok      libclang                               find_library("clang") -> libclang.so.23.1
ok      python3-yaml
ok      glslangValidator                       Glslang Version: 11:16.4.0
ok      pkg-config-expat                       2.8.5
ok      pkg-config-zlib                        1.3.2
```

## Registering a prepared machine with ccfarm

After `--check` is green, `--install` prints the values worth registering:

- `ccfarm env set yakogl CC_aarch64_unknown_none <compiler>` — needed when
  clang is not on PATH (a machine with only a versioned clang, such as
  `clang-23`, names it here; `build.rs` also accepts
  `CC_aarch64-unknown-none` and `TARGET_CC`).
- `export LIBCLANG_PATH=<dir>` — needed when `find_library('clang')` fails
  but a `libclang.so*` exists on disk; bindgen reads `LIBCLANG_PATH` first.

Both are printed only; applying them stays an owner (or ccfarm) decision.
