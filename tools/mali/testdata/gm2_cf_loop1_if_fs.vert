#version 100
/* gm2_cf_loop1_if_fs: the vertex stage writes the two varyings the fragment
 * case reads, from the attributes. */
attribute vec4 a_pos;
attribute vec3 a_nrm;
varying vec3 v_nrm;
varying vec3 v_view;
void main() {
    v_nrm = a_nrm;
    v_view = a_pos.xyz;
    gl_Position = a_pos;
}
