// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Each back end's pure, host-testable words for a 4x multisample
//! framebuffer object (no state, no back-end types).
//!
//! The tree lives outside the back-end directories (`src/mali`, `src/pvr`,
//! `src/v3d`, `src/vivante`) because other plans hold them; each `*-msaa-fbo`
//! item wires its module from its back end
//! (docs/notes/es3-msaa-renderbuffers.md (c), (f), (f5)). Nothing calls it yet.

pub mod mali;
