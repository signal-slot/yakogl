// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Which uniforms make a highp fragment shader "mixed" (docs/notes/mali-indexed-locals.md
//! section 4 item 2): only the float ones. Mesa's `LowerPrecisionFloat16Uniforms`
//! turns a float uniform not declared highp into an fp16 one; an int, uint or bool
//! uniform stays one 32-bit word at any precision (case 80's `ICMP_OR.s32 .., u4.w1`).

use super::tests::compile;
use super::MaliError;
use crate::shaderc::online::codegen::{self, CompileError};
use crate::shaderc::SHC_STAGE_FRAGMENT;

/// The Mali gap a fragment shader meets first, None if it compiles.
fn verdict(src: &str) -> Option<MaliError> {
    match compile(SHC_STAGE_FRAGMENT, src) {
        Ok(_) => None,
        Err(CompileError::BackEnd(codegen::BackEndError::Mali(e))) => Some(e),
        Err(e) => panic!("{src}: not a Mali gap: {e:?}"),
    }
}

/// A default-precision (mediump) int uniform compared in a highp shader.
const INT_COMPARE: &str = "precision highp float; uniform vec4 u_a; uniform int u_n; \
     void main(){ vec4 c = u_a; if (u_n > 2) c = u_a * u_a; gl_FragColor = c; }";

#[test]
fn a_default_precision_int_uniform_does_not_make_a_highp_shader_mixed() {
    assert_ne!(verdict(INT_COMPARE), Some(MaliError::MixedPrecision));
}

#[test]
fn a_bool_uniform_does_not_either() {
    let b = "precision highp float; uniform vec4 u_a; uniform bool u_b; \
             void main(){ vec4 c = u_a; if (u_b) c = u_a * u_a; gl_FragColor = c; }";
    assert_ne!(verdict(b), Some(MaliError::MixedPrecision));
}

#[test]
fn a_mediump_float_uniform_in_a_highp_shader_still_does() {
    let f = "precision highp float; uniform mediump vec4 u_a; uniform vec4 u_b; \
             void main(){ gl_FragColor = u_a * u_b; }";
    assert_eq!(verdict(f), Some(MaliError::MixedPrecision));
    /* beside an int uniform, the float one decides */
    let g = "precision highp float; uniform mediump vec4 u_a; uniform int u_n; \
             void main(){ vec4 c = u_a; if (u_n > 2) c = u_a * u_a; gl_FragColor = c; }";
    assert_eq!(verdict(g), Some(MaliError::MixedPrecision));
}

/// Case 80 still refuses first at the indexed local, not at the uniform.
#[test]
fn case_80_refuses_at_the_indexed_local() {
    let src = include_str!("../../../../tools/mali/testdata/other_80_matdyn.frag");
    assert_eq!(verdict(src), Some(MaliError::IndexedLocal));
}
