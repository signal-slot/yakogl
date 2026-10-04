// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GLSL ES link rules, for the two sources of one program.
//!
//! `src/shaderc/offline.rs`'s `link()` pairs the stages from the *compiled*
//! reflection: it sees a varying's location, component count and name, and it
//! patches the fragment shader's `vtxout_off` from the vertex shader's
//! outputs. That is the wiring, and it is the same code on both back ends.
//! What it cannot see is what the ES source said — `mediump vec3` and
//! `highp vec3` compile to the same thing, and a uniform of a different type
//! in each stage is two independent blocks by the time pco_shc is done — so
//! `dEQP-GLES2.functional.shaders.linkage.*` needs a check with both ES texts
//! in hand, which is what this is.
//!
//! It is one call now. This module used to read the declarations out of the
//! source text with regular expressions; [`crate::es100::link`] runs
//! `glslangValidator -l` over both texts, which is a real parser applying the
//! same front end that compiled each stage, and then adds the two ES rules
//! glslang does not apply (a varying the fragment shader uses and the vertex
//! shader never declares, and a uniform at a different precision). Keeping
//! both would have been two implementations of one specification, free to
//! drift; the tests below are the regex version's, kept because the rules
//! they pin are right whoever checks them.

use crate::es100;
use crate::{Error, R};

/// Check one program's two ES sources against the ES link rules.
/// `Ok(())` when they link; `Error::Exit` carries the log the application
/// reads back through `glGetProgramInfoLog`.
///
/// An `Error::Fatal` — glslang missing or unrunnable — is the service
/// failing, not a link failure, and the caller must report it as such.
///
/// `max_varying_vectors` is the client GPU's GL_MAX_VARYING_VECTORS, which
/// the varying budget packs against (the request says it; see
/// [`es100::link_for`]).
pub fn check_for(vert: &str, frag: &str, max_varying_vectors: usize) -> R<()> {
    // The GLSL ES link rules apply to GLSL ES. A source that is not ES is not
    // one they have anything to say about, and running it through them does
    // not merely waste time: `es100::link` invokes glslang in a GL/ES profile,
    // and our own internal and test shaders are GLSL 450 written for SPIR-V
    // with Vulkan-only qualifiers in them -- `layout(push_constant) uniform U
    // { mat4 u_mvp; };` is gl_test case 1's vertex shader. glslang refuses
    // that outside Vulkan, so the pair comes back as a LINK ERROR for a
    // program that links perfectly well and that the driver has been running
    // all along.
    //
    // That is what happened on the i.MX8MP the hour the GC7000 link check
    // started calling this: gl_test case 1 failed at link with
    // "'push_constant' : only allowed when using GLSL for Vulkan", while the
    // same case passed on the offline-table pair minutes apart. The defect
    // was here and older than the caller; it had simply never been reached,
    // because no pipeline asked this to link a non-ES pair before.
    //
    // So: ES pairs get the ES rules, and anything else is not this function's
    // business. It is not a silent pass -- a non-ES source has already been
    // through its own compiler by this point, and the GPU-specific wiring
    // check (tools/shaderc_server.py's vivante pipeline) still runs on it.
    if !is_es(vert) || !is_es(frag) {
        return Ok(());
    }
    let scratch = es100::Scratch::new()?;
    match es100::link_for(&scratch, vert, frag, max_varying_vectors)? {
        Ok(()) => Ok(()),
        Err(d) => Err(Error::Exit(d.log.trim_end().to_string())),
    }
}

/// Is this GLSL ES?
///
/// `#version 100` and `#version 300 es` are; a version directive without `es`
/// is not; and a source with no directive at all is GLSL ES 1.00 by
/// GLSL ES 1.00 section 3.3, which is what an application that writes
/// `attribute vec4 aPos;` and nothing else relies on.
fn is_es(src: &str) -> bool {
    for line in src.lines() {
        let t = line.trim_start();
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        if let Some(rest) = t.strip_prefix("#version") {
            // "300 es", "100", "450 core", "450"
            return rest.split_whitespace().any(|w| w == "es") || rest.trim() == "100";
        }
        // The first thing that is not blank, a comment or the version
        // directive: there was no directive, so ES 1.00 by the spec.
        if !t.starts_with("/*") && !t.starts_with('*') {
            return true;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    //! The rules these pin are GL ES 2.0's and did not change with the
    //! collapse; the message texts are glslang's now, except for the two
    //! rules `es100::link` adds because glslang does not apply them.
    use super::*;

    /// Which sources the ES link rules apply to.
    ///
    /// This is the guard that was missing for one hour on 2026-09-22, and the
    /// cost of getting it wrong is not a wasted glslang run: our own internal
    /// and test shaders are GLSL 450 with Vulkan-only qualifiers, and running
    /// them through a GL/ES profile turns a program the driver has been
    /// running all along into a link failure on the board.
    #[test]
    fn the_es_rules_apply_to_es_sources_only() {
        // ES, and therefore checked.
        assert!(is_es("#version 100\nattribute vec4 p;\n"));
        assert!(is_es("#version 300 es\nin vec4 p;\n"));
        // No directive at all is ES 1.00 by GLSL ES 1.00 section 3.3, which is
        // what an application writing plain `attribute vec4 aPos;` relies on.
        assert!(is_es("attribute vec4 aPos;\nvoid main(){}\n"));
        assert!(is_es("// a comment first\n\n#version 100\n"));
        // Not ES, and therefore not this function's business.
        assert!(!is_es("#version 450\nlayout(push_constant) uniform U { mat4 m; };\n"));
        assert!(!is_es("#version 450 core\n"));
        assert!(!is_es("#version 330\n"));
    }

    /// gl_test case 1's own pair, verbatim, as the regression it was.
    #[test]
    fn a_glsl_450_pair_with_a_push_constant_block_links() {
        let vs = "#version 450\nlayout(push_constant) uniform U { mat4 u_mvp; };\n\
                  layout(location = 0) in vec4 a_pos;\nvoid main() { gl_Position = u_mvp * a_pos; }\n";
        let fs = "#version 450\nlayout(push_constant) uniform U { vec4 u_color; };\n\
                  layout(location = 0) out vec4 o_color;\nvoid main() { o_color = u_color; }\n";
        assert_eq!(log(vs, fs), "", "a GLSL 450 pair must not be judged by the ES link rules");
    }

    /// The link log, or "" when the pair links.
    fn log(vert: &str, frag: &str) -> String {
        match check_for(vert, frag, yakogl::gl::context::MAX_VARYING_VECTORS) {
            Ok(()) => String::new(),
            Err(Error::Exit(m)) => m,
            Err(e) => panic!("{e:?}"),
        }
    }

    const VS: &str = "\
attribute vec4 aPos;
varying vec2 vUV;
uniform mat4 uMVP;
void main() { vUV = aPos.xy; gl_Position = uMVP * aPos; }
";

    #[test]
    fn a_matching_pair_links() {
        let fs = "precision mediump float;\nvarying vec2 vUV;\nvoid main() { gl_FragColor = vec4(vUV, 0.0, 1.0); }\n";
        assert_eq!(log(VS, fs), "");
    }

    #[test]
    fn a_varying_the_vertex_shader_does_not_write_is_a_link_error() {
        let fs =
            "precision mediump float;\nvarying vec2 vOther;\nvoid main() { gl_FragColor = vec4(vOther, 0.0, 1.0); }\n";
        /* This is the rule glslang does not apply -- it models an unmatched
         * fragment input as legal, which GLSL 4.50 is -- so the message is
         * es100::unmatched_varying's own. */
        assert!(
            log(VS, fs).contains("varying 'vOther' is used by the fragment shader but not declared"),
            "{}",
            log(VS, fs)
        );
    }

    #[test]
    fn an_unread_varying_may_be_left_unwritten() {
        /* ES 2.0 only makes a *statically used* varying a link error */
        let fs = "precision mediump float;\nvarying vec2 vUnused;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
        assert_eq!(log(VS, fs), "");
    }

    #[test]
    fn a_varying_of_a_different_type_is_a_link_error() {
        let fs = "precision mediump float;\nvarying vec3 vUV;\nvoid main() { gl_FragColor = vec4(vUV, 1.0); }\n";
        let m = log(VS, fs);
        assert!(m.contains("Types must match"), "{m}");
        assert!(m.contains("vec2 vUV") && m.contains("vec3 vUV"), "both stages are named: {m}");
    }

    #[test]
    fn a_uniform_of_a_different_type_is_a_link_error() {
        let fs = "precision mediump float;\nuniform vec4 uMVP;\nvarying vec2 vUV;\n\
                  void main() { gl_FragColor = uMVP + vec4(vUV, 0.0, 0.0); }\n";
        let m = log(VS, fs);
        assert!(m.contains("Types must match"), "{m}");
        assert!(m.contains("mat4x4 uMVP") && m.contains("vec4 uMVP"), "both stages are named: {m}");
    }

    #[test]
    fn a_uniform_of_a_different_precision_is_a_link_error() {
        /* the vertex language defaults float to highp, so a mediump uniform
         * in the fragment shader does not match unless it says highp */
        let fs = "precision mediump float;\nuniform mat4 uMVP;\nvarying vec2 vUV;\n\
                  void main() { gl_FragColor = uMVP * vec4(vUV, 0.0, 1.0); }\n";
        assert!(log(VS, fs).contains("Precision qualifiers must match"), "{}", log(VS, fs));

        let ok = "precision mediump float;\nuniform highp mat4 uMVP;\nvarying vec2 vUV;\n\
                  void main() { gl_FragColor = uMVP * vec4(vUV, 0.0, 1.0); }\n";
        assert_eq!(log(VS, ok), "");
    }

    #[test]
    fn an_array_uniform_must_agree_in_size() {
        let vs = "uniform vec4 uC[4];\nvoid main() { gl_Position = uC[0]; }\n";
        let fs = "precision highp float;\nuniform vec4 uC[3];\nvoid main() { gl_FragColor = uC[0]; }\n";
        let m = log(vs, fs);
        assert!(m.contains("Array sizes must be compatible"), "{m}");
        assert!(m.contains("uC[4]") && m.contains("uC[3]"), "both stages are named: {m}");
        let same = "precision highp float;\nuniform vec4 uC[4];\nvoid main() { gl_FragColor = uC[0]; }\n";
        assert_eq!(log(vs, same), "");
    }

    #[test]
    fn a_declaration_in_a_comment_is_not_a_declaration() {
        let fs = "precision mediump float;\n// varying vec3 vUV;\nvarying vec2 vUV;\n\
                  void main() { gl_FragColor = vec4(vUV, 0.0, 1.0); }\n";
        assert_eq!(log(VS, fs), "");
    }

    #[test]
    fn es_300_in_out_varyings_are_paired_too() {
        let vs = "#version 300 es\nin vec4 aPos;\nout vec2 vUV;\nvoid main() { vUV = aPos.xy; gl_Position = aPos; }\n";
        let fs = "#version 300 es\nprecision mediump float;\nin vec3 vUV;\nout vec4 oC;\n\
                  void main() { oC = vec4(vUV, 1.0); }\n";
        assert!(log(vs, fs).contains("Types must match"), "{}", log(vs, fs));
    }

    #[test]
    fn the_sampler_default_precision_is_lowp_in_both_stages() {
        let vs = "uniform sampler2D uTex;\nvoid main() { gl_Position = texture2D(uTex, vec2(0.0)); }\n";
        let fs = "precision mediump float;\nuniform sampler2D uTex;\n\
                  void main() { gl_FragColor = texture2D(uTex, vec2(0.0)); }\n";
        assert_eq!(log(vs, fs), "");
    }

    #[test]
    fn bool_uniforms_carry_no_precision_to_mismatch() {
        let vs = "uniform bool uOn;\nvoid main() { gl_Position = vec4(uOn ? 1.0 : 0.0); }\n";
        let fs = "precision mediump float;\nuniform bool uOn;\nvoid main() { gl_FragColor = vec4(uOn ? 1.0 : 0.0); }\n";
        assert_eq!(log(vs, fs), "");
    }

    #[test]
    fn a_varying_array_must_agree_in_size_in_every_direction() {
        /* The rule shaderc-remote asked to see survive the collapse, and the
         * loosest case with it: an unsubscripted varying against `[1]`. */
        let v2 = "attribute highp vec4 p;\nvarying mediump vec4 v[2];\nvoid main(){ v[0]=p; v[1]=p; gl_Position=p; }\n";
        let f3 =
            "precision mediump float;\nvarying mediump vec4 v[3];\nvoid main(){ gl_FragColor = v[0]+v[1]+v[2]; }\n";
        let f_plain = "precision mediump float;\nvarying mediump vec4 v;\nvoid main(){ gl_FragColor = v; }\n";
        let v_plain = "attribute highp vec4 p;\nvarying mediump vec4 v;\nvoid main(){ v=p; gl_Position=p; }\n";
        let f1 = "precision mediump float;\nvarying mediump vec4 v[1];\nvoid main(){ gl_FragColor = v[0]; }\n";
        for (a, b, what) in
            [(v2, f3, "[2] vs [3]"), (v2, f_plain, "[2] vs unsubscripted"), (v_plain, f1, "unsubscripted vs [1]")]
        {
            assert!(log(a, b).contains("Array sizes must be compatible"), "{what}: {}", log(a, b));
        }
    }

    #[test]
    fn a_sampler_is_lowp_in_the_vertex_stage_too() {
        /* GLSL ES gives sampler2D a lowp default in BOTH stages. Resolving
         * the vertex default as highp -- as the first version of
         * es100::default_precision did -- makes a link error out of every
         * program that samples in both stages. */
        let vs = "uniform sampler2D uTex;\nvoid main() { gl_Position = texture2D(uTex, vec2(0.0)); }\n";
        let fs = "precision mediump float;\nuniform sampler2D uTex;\nvoid main() { gl_FragColor = texture2D(uTex, vec2(0.0)); }\n";
        assert_eq!(log(vs, fs), "");
    }

    #[test]
    fn a_struct_uniform_at_two_precisions_is_a_link_error() {
        /* dEQP's shaders.linkage.uniform_struct_precision_conflict_*: glslang
         * compares a plain uniform's precision but not a struct member's, so
         * this is the other rule es100::link adds. */
        let vs = "attribute highp vec4 p;\nstruct S { highp float a; };\nuniform S val;\nvoid main() { gl_Position = p * val.a; }\n";
        let fs = "precision mediump float;\nstruct S { mediump float a; };\nuniform S val;\nvoid main() { gl_FragColor = vec4(val.a); }\n";
        assert!(log(vs, fs).contains("val.a"), "{}", log(vs, fs));
    }
}
