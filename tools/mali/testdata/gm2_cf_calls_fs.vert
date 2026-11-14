#version 100
/* gm2_cf_calls_fs: the vertex stage writes the varying the fragment case
 * reads. */
attribute vec4 a_pos;
varying vec2 v_uv;
void main() {
    v_uv = a_pos.xy;
    gl_Position = a_pos;
}
