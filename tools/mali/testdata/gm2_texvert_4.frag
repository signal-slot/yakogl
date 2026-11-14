#version 100
/* #32 plan #367 ref-tex-vertex: fragment shader of the gm2_texvert_1, _2 and _4 pairs
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; see
 * docs/notes/mali-vs-texture-lanes.md). */
precision mediump float;
varying vec4 v_a;
varying vec4 v_b;
void main() {
    gl_FragColor = v_a + v_b;
}
