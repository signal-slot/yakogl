#version 100
/* #1384/#1385 plan #153 study-vstex: minimal vertex shader, one sampler, a look-up at a coordinate with one constant lane (the t lane constant)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; see
 * docs/notes/mali-vs-texture-lanes.md). */
attribute vec4 a_pos;
uniform sampler2D u_s0;
varying vec4 v_c;
void main() {
    v_c = texture2DLod(u_s0, vec2(a_pos.x, 0.5), 0.0);
    gl_Position = a_pos;
}
