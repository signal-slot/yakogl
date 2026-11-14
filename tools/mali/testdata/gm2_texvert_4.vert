#version 100
/* #32 plan #367 ref-tex-vertex: two vertex look-ups of one sampler at one coordinate (one TEX_SINGLE in Mesa after nir_opt_cse), their results used apart
 * (original reduction)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; family TEX_VERTEX, see
 * docs/notes/mali-vs-texture-lanes.md). */
attribute vec4 a_pos;
attribute vec2 a_uv;
uniform sampler2D u_s0;
varying vec4 v_a;
varying vec4 v_b;
void main() {
    v_a = texture2D(u_s0, a_uv);
    v_b = texture2D(u_s0, a_uv) * vec4(0.5, 1.0, 1.0, 1.0) + a_pos;
    gl_Position = a_pos;
}
