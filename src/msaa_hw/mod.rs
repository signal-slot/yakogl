// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Each back end's pure, host-testable words for a 4x multisample
//! framebuffer object (no state, no back-end types).
//!
//! The tree lives outside the back-end directories (`src/mali`, `src/pvr`,
//! `src/v3d`, `src/vivante`) because other plans hold them; each `*-msaa-fbo`
//! item wires its module from its back end
//! (docs/notes/es3-msaa-renderbuffers.md (c), (f), (f5)). Nothing calls it yet.
//! `gc` is the GC7000UL's (i.MX8MP, `src/vivante`): scratch geometry, RS
//! resolve words and the measured arm-19 state ((c3), (f3)).
//! `pvr` is the PowerVR's (AM62P, `src/pvr`): the ISP 4x mode, tiles in
//! flight, the PBE downscale resolve and the sample-store geometry ((c1), (f2)).
//! `v3d` is the V3D 4.2's (Pi 4, `src/v3d`): the 4x tile size, internal bpp,
//! per-RT sample-buffer layout and the resolve, per-sample and sample-0 store
//! words ((c2), (f1)).

pub mod gc;
pub mod mali;
pub mod pvr;
pub mod v3d;
