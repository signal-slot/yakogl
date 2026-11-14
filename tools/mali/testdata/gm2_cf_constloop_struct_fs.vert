#version 100
/* gm2_cf_constloop_struct_fs: the vertex stage writes the three varyings the fragment
 * case reads, each from an attribute. */
attribute vec4 a_pos;
attribute vec3 a_nrm;
varying vec3 v_nrm;
varying vec4 v_eye;
varying vec3 v_dir;
void main() {
    v_nrm = a_nrm;
    v_eye = a_pos;
    v_dir = a_pos.zyx;
    gl_Position = a_pos;
}
