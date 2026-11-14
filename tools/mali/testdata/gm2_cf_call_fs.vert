#version 100
/* gm2_cf_call_fs: the vertex stage of a fragment case (a position only). */
attribute vec4 a_pos;
void main() {
    gl_Position = a_pos;
}
