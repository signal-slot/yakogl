// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The triangle sample's two shader texts, the one place they are named:
//! the check crate (`lib.rs`) re-exports them and the sample draws them;
//! YakoGL's online compiler compiles them in a host test
//! (src/shaderc/online/triangle_sample_tests.rs, through `#[path]`), which
//! is the gate that they are GLSL ES 1.00 our compiler builds, not only
//! texts an offline table holds.
//!
//! tools/shaders/gl_test_es100/ holds the ES 1.00 twins of gl_test's
//! passthrough and interpolated-colour programs. Not tools/shaders/
//! passthrough.vert and interp_color.frag: those are `#version 450`
//! (desktop/Vulkan GLSL), which an OpenGL ES 2.0 sample must not hand to
//! glShaderSource (the Pi 4 compiler refused them, 92a360b8).

/// a_pos and a_color through, v_color out; NUL-terminated.
pub const VS_SRC: &str = concat!(include_str!("../../../../../../tools/shaders/gl_test_es100/passthrough.vert"), "\0");
/// gl_FragColor = v_color; NUL-terminated.
pub const FS_SRC: &str = concat!(include_str!("../../../../../../tools/shaders/gl_test_es100/interp_color.frag"), "\0");
