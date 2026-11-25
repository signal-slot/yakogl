// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR test knobs (`test-knobs` feature), in the `pvr_debug_*` namespace
//! the gate reads (tools/check.sh "no test knob in the artefact").
#![cfg(feature = "test-knobs")]

use crate::backend::{draw_targets, GLES_MAX_TARGETS, KNOB_DRAW_TARGETS};
use core::sync::atomic::Ordering;

/// Make the library answer `n` colour targets (1 to `GLES_MAX_TARGETS`) as
/// GL_MAX_DRAW_BUFFERS, GL_MAX_COLOR_ATTACHMENTS and `gl_MaxDrawBuffers`
/// (docs/notes/mrt4-deqp-cases.md: the deqp sample's `CONFIG_PVR_DEQP_MRT4`
/// calls it with 4). 0 restores the build's default. Returns the count
/// answered before the call; an out-of-range `n` is refused and changes
/// nothing. Set it before the first shader is compiled.
#[no_mangle]
pub extern "C" fn pvr_debug_set_draw_targets(n: i32) -> i32 {
    let was = draw_targets() as i32;
    if n < 0 || n as usize > GLES_MAX_TARGETS {
        return was;
    }
    KNOB_DRAW_TARGETS.store(n as u8, Ordering::Relaxed);
    was
}
