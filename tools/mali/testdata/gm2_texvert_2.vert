#version 100
/* #32 plan #367 ref-tex-vertex: two vertex look-ups of two samplers (a varying each; original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_VERTEX, see
 * docs/notes/mali-vs-texture-lanes.md). */
attribute vec4 a_pos;
attribute vec2 a_uv;
uniform sampler2D u_s0;
uniform sampler2D u_s1;
varying vec4 v_a;
varying vec4 v_b;
void main() {
    v_a = texture2D(u_s0, a_uv);
    v_b = texture2D(u_s1, a_uv);
    gl_Position = a_pos;
}
