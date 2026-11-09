#version 100
/* #33 plan #336 vs-lookups: two vertex look-ups of two samplers out of three
 * declared, the higher binding read first: each look-up's descriptor index is
 * its sampler's rank by binding among the samplers read (u_s2 1, u_s0 0)
 * (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1; see
 * docs/notes/mali-vs-lookups.md). */
attribute vec4 a_pos;
uniform sampler2D u_s0;
uniform sampler2D u_s1;
uniform sampler2D u_s2;
uniform vec4 u_w;
varying vec4 v_c;
void main() {
    gl_Position = a_pos;
    vec2 c = vec2(0.5);
    v_c = u_w.x * texture2DLod(u_s2, c, 0.0) + u_w.y * texture2DLod(u_s0, c, 0.0);
}
