#version 100
/* The passthrough VS (gl_test_es100/passthrough.vert) under the name that
 * pairs it with tri_fragcoord.frag: v3d_triangle's gl_FragCoord variant
 * (TRI_FRAGCOORD). */
attribute vec4 a_pos;
attribute vec4 a_color;
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
}
