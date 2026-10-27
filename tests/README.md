<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Host tests

The tests in this directory run on the host, without a GPU. Nothing is
rendered here, so they check API semantics, compiler output and the command
streams the back ends build. Rendering is tested on silicon by
`driver/tools/pvrlab/gl_test.c`, which the Zephyr gl_test images run on all
three boards.

## Cargo integration tests

Every `*.rs` file here is a cargo integration test of the `yakogl` crate. The
gate (`tools/check.sh`) runs them with `cargo test` for each feature set in
its `CONFIGS` array; a test file that needs one GPU or the online compiler
says so in its `#![cfg(...)]` line. To run one:

```sh
cargo test --no-default-features --features host-test,gpu-powervr --test <name>
```

Shared code is in `common/` (an interpreter of encoded GC7000 instructions,
and dEQP-style helpers) and `usc_exec/` (an interpreter of the PowerVR USC
subset our builder emits). `reference/` holds the Mesa reference tables
(`blobs.rs`, `vivante_blobs.rs` and the internal programs), used only for
measurement. `fixtures/` holds pinned inputs and expected outputs; the
tests name the files they read.

## gl_test checks in C

`gltest_hdr_probe.c` and `gltest_viewwin_order.c` include
`driver/tools/pvrlab/gl_test.c` and check one piece of it on the host. Each
file's header has its build line. The gate builds and runs the first. The
walk's count (a skip is not a failure, the first two failures are named) is
the gl_test runner's, `driver/tools/pvrlab/runner`, tested with
`cargo test -p yakogl-gltest-runner`.

## negative_api

`negative_api` runs dEQP-GLES2 `functional.negative_api` (246 cases in the AOSP
mustpass). The case bodies are not rewritten: `negative_api/extract.py` copies
them out of the conformance suite's own sources, and `negative_api/deqp_shim.hpp`
supplies the few dEQP helpers they call. What runs here is what the suite runs.
It is a CMake target and the gate does not build it.

```sh
cmake -S . -B build/host -DYAKOGL_OSAL=linux -DYAKOGL_SHADERC=offline \
      -DYAKOGL_TESTS=ON -DYAKOGL_CTS_DIR=/path/to/VK-GL-CTS
cmake --build build/host --target negative_api
./build/host/tests/negative_api            # all cases
./build/host/tests/negative_api texture.   # one group
```

The exit status is 0 when every selected case passes.
