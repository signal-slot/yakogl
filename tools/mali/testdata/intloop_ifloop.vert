#version 100
/* The passthrough VS (gl_test_es100/passthrough.vert) under the name that
 * pairs it with tri_int.frag: v3d_triangle's int variant (TRI_INT). */
attribute vec4 a_pos;
attribute vec4 a_color;
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
}
