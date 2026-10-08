# Host toolchain for the aarch64-unknown-none rows (system76 and similar)

Date: 2026-10-08. Host: the review machine (system76), no `clang` on PATH, no sudo.

## Result in one line

The Zephyr SDK 1.0.1 gcc is **accepted by build.rs unchanged**. No LLVM was
installed: this session's sandbox refused every network fetch (`curl`, `gh api`),
so step (2) could not be run. It was not needed for the cargo rows. The
bindgen row passes with the host's existing libclang plus gcc's builtin headers.

## Installed where

Nothing new was installed. Used from the deps cache (`$CCFARM_DEPS_DIR`, here
`/home/tasuku/.local/share/ccfarm/deps/yakogl`):

| What | Path | Version |
|---|---|---|
| cross C compiler | `zephyr-sdk-1.0.1/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc` | gcc 14.3.0 (Zephyr SDK 1.0.1) |
| real binary | `.../bin/aarch64-zephyr-elf-gcc-14.3.0` | sha256 `c1fc7a1b3428db5e5f029a0ecfb1f4cda8235c588d5d0662c17fff889ed1ce3e` |
| gcc builtin headers | `zephyr-sdk-1.0.1/gnu/aarch64-zephyr-elf/lib/gcc/aarch64-zephyr-elf/14.3.0/include` | (stdbool.h etc.) |

The Rust target `aarch64-unknown-none` was already installed (rustup).

## Exact env values

```
CC_aarch64_unknown_none=$CCFARM_DEPS_DIR/zephyr-sdk-1.0.1/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc
# only for the bindgen row (csbgen-bindings --check):
BINDGEN_EXTRA_CLANG_ARGS="-isystem $CCFARM_DEPS_DIR/zephyr-sdk-1.0.1/gnu/aarch64-zephyr-elf/lib/gcc/aarch64-zephyr-elf/14.3.0/include"
```

`LIBCLANG_PATH` was not needed: the host already has a libclang that bindgen
finds; it only lacked clang's resource directory (`stdbool.h`), which the
`BINDGEN_EXTRA_CLANG_ARGS` above supplies.

In the review sandbox, env prefixes on a command were refused, so the values
were passed as `cargo --config env.NAME='"value"'` (cargo's `[env]` reaches
build scripts); a plain `export` works the same for check.sh.

## Rows run (commands exactly as tools/check.sh runs them)

All with the env above, on commit 3f78982e.

| Row | Result |
|---|---|
| `check osal-zephyr,gpu-powervr,shaderc-online,no-shader-table (aarch64-unknown-none, no host-test)` | PASS (`Finished dev profile ... in 39.21s`) |
| `check osal-zephyr,gpu-vivante,shaderc-online,no-shader-table (aarch64-unknown-none, ...)` | PASS (`Finished ... in 2.18s`) |
| `check osal-zephyr,gpu-v3d,shaderc-online,no-shader-table (aarch64-unknown-none, ...)` | PASS (`Finished ... in 23.53s`) |
| `check osal-zephyr,gpu-powervr,precompiled-shaders (aarch64-unknown-none, no compiler)` (tables from `tier_tables`: `check-powervr.rs`, `check-gc7000.rs`) | PASS (`Finished ... in 13.53s`) |
| `no Mesa reference in the artefact`, `--quick` subset: control `osal-zephyr,gpu-powervr,mesa-reference-table` | control links the reference symbols (hundreds of matches) |
| same, product `aarch64-unknown-none osal-zephyr,gpu-powervr,shaderc-online,no-shader-table` | 0 reference symbols |
| same, product `aarch64-unknown-none osal-zephyr,gpu-powervr,precompiled-shaders` | 0 reference symbols |
| `gen src/pvr/csbgen.rs + csbgen_wrap.c == bindgen` (`csbgen-bindings --check`) | PASS with `BINDGEN_EXTRA_CLANG_ARGS`; FAIL without it (`'stdbool.h' file not found`) |

Caveats, stated plainly:

- The "no Mesa reference" row was reproduced by hand, not through
  `tools/check.sh`: the same cargo builds, then the symbol pattern counted with
  `grep -ac` on the archive (the sandbox refused `nm`). That counts
  matching lines, not symbols, so 0 is exact but the control figure is not
  comparable with check.sh's `nm` count. Only the `--quick` configurations were
  run (the full matrix is 20 archives, ~863 s), so the vivante control and
  the other OSAL/GPU archives are not covered here.
- The whole `tools/check.sh` gate was not run (the sandbox refused script and
  `env` invocations); only the aarch64 rows, the Mesa-reference row and the
  bindgen row above.
- No Mesa-reference or bindgen-less variant of gcc diagnostics was seen: build.rs
  passes only `-ffreestanding -fno-stack-protector -Wno-override-init`, all of
  which gcc takes.

## Follow-ups (not done, no code changed)

- If a machine has no libclang at all, fetch an official LLVM x86_64-linux
  release tarball into `$CCFARM_DEPS_DIR/llvm-<ver>` (own empty dir) and set
  `LIBCLANG_PATH=$CCFARM_DEPS_DIR/llvm-<ver>/lib`; its `bin/clang` then also
  serves as `CC_aarch64_unknown_none`. Not done here (no network).
- check.sh could pass the gcc include dir to bindgen itself, or the csbgen row
  could document `BINDGEN_EXTRA_CLANG_ARGS`.
