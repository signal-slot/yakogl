// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// Fixture: the shape of the MaliError enum and a src/mali constant.
pub enum MaliError {
    /// A compare nobody draws.
    CompareForm,
    ControlFlow,
    Arithmetic,
}

pub const NOT_BUILT_EXAMPLE: &str = "mali: an example is not built yet";
