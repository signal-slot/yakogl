#version 100
/* #32 plan #367 ref-tex-vertex: a vertex look-up whose result feeds gl_Position, a second look-up feeds a varying
 * (the terrain 18bc shape, original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_VERTEX, see
 * docs/notes/mali-vs-texture-lanes.md). */
attribute vec4 a_pos;
attribute vec3 a_nrm;
attribute vec2 a_uv;
uniform sampler2D u_disp;
uniform sampler2D u_nrm;
uniform float u_scale;
uniform float u_bias;
varying vec3 v_n;
void main() {
    vec3 dv = texture2D(u_disp, a_uv).xyz;
    float df = u_scale * dv.x + u_bias;
    gl_Position = vec4(a_nrm * df, 0.0) + a_pos;
    v_n = texture2D(u_nrm, a_uv).xyz * 2.0 - 1.0;
}
