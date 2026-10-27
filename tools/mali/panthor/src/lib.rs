// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The development OSAL for the Mali-G610 port: YakoGL's GPU access on the
//! NanoPC-T6 under Armbian, through panthor's uAPI
//! (driver/docs/nanopc-t6-mali.md, milestones L1-L3, the layout table's
//! "Development OSAL on Linux" row). It never ships in an image: the product
//! path is our own kernel side on Zephyr (Z1-Z4).
//!
//! What it gives: [`dev::Device`] (open, the device queries),
//! [`dev::Vm`], [`dev::Bo`] (create, CPU map, bind at a GPU VA),
//! [`dev::Group`] with one queue, [`dev::TilerHeap`], submit returning a
//! [`dev::Fence`] and the wait; [`cs`] encodes the command stream
//! instructions it needs; [`pandecode`] reads a Mesa trace with memory
//! dumps for `panthor-replay`; [`l1`] builds YakoGL's own triangle frame
//! (src/mali/frame.rs) at the VAs of the buffers it is given, with the
//! board's shader programs ([`tri_shaders`]), for `panthor-frame`;
//! [`osal`] is the `OsalOps` table (src/osal/mod.rs, the Mali part in
//! src/osal/mali.rs) the library's `gpu-mali` backend runs on, for
//! `panthor-gl` (L2) and `panthor-gltest` (L3: gl_test.c, the suite every
//! board runs, which `build.rs` compiles as its Mali arm).
//!
//! Build for the board (static, so the board's glibc 2.41 does not have to
//! match the host's; the linker is the host's aarch64 cross gcc):
//!
//! ```text
//! CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
//! CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C target-feature=+crt-static" \
//!   cargo build -p yakogl-mali-panthor --release --target aarch64-unknown-linux-gnu
//! ```
//!
//! `tools/mali/panthor/run-on-board.sh` does that, copies the binary to
//! `$BOARD_HOST` and runs it there.

pub mod cs;
pub mod dev;
pub mod l1;
pub mod osal;
pub mod pandecode;
pub mod tri_shaders;
pub mod uapi;

/// `git describe --always --dirty` of the tree this was built from
/// ("unknown" outside a git checkout).
pub const BUILD: &str = env!("YAKOGL_MALI_PANTHOR_BUILD");

/// The same stamp between NUL bytes, for `run-on-board.sh`, which looks for
/// `\0yakogl-build=<describe>\0` in the binary (a plain `BUILD` sits inside
/// concatenated read-only strings, where an exact match cannot be told from
/// a prefix such as the same hash without `-dirty`).
#[used]
pub static BUILD_MARK: &str = concat!("\0yakogl-build=", env!("YAKOGL_MALI_PANTHOR_BUILD"), "\0");
