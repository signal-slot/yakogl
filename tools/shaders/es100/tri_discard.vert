#version 100
/* The passthrough VS (gl_test_es100/passthrough.vert) under the name that
 * pairs it with tri_discard.frag: v3d_triangle's discard variant (TRI_DISCARD). */
attribute vec4 a_pos;
attribute vec4 a_color;
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
}
