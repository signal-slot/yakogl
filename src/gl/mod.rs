// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The OpenGL ES / EGL front end: the `gl*` and `egl*` entry points, the
//! context and object model, and the draw resolver that turns the current
//! state into one [`crate::backend::GlesDraw`].
//!
//! Conventions shared by the modules below:
//!  - an entry point takes the current context ONCE (`context::current()`)
//!    and passes `&mut GlContext` down; helpers never fetch it again;
//!  - object handles are 1-based indices into the context's fixed tables
//!    (`GlContext::buffer(h)` etc. return `None` for 0 / out of range / free);
//!  - errors are recorded with `c.set_error(GL_INVALID_..., "glEntryPoint")`.

pub mod consts;
pub mod egl_consts;
pub mod types;

pub mod context;

pub mod buffer;
pub mod draw;
pub mod egl;
pub mod es3;
pub mod es3_formats;
pub mod etc1;
pub mod framebuffer;
pub mod names;
pub mod native_cube;
pub mod program;
pub mod shader;
pub mod state;
pub mod tf;
pub mod texture;
pub mod vao;
pub mod vertex;

#[cfg(test)]
mod es3_format_tests;
#[cfg(test)]
mod es3_level_tests;
#[cfg(test)]
mod pool_tests;
#[cfg(test)]
mod tf_tests;
