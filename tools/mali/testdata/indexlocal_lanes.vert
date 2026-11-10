#version 100
/* Passthrough VS: position + one colour varying. ES 1.00 twin of
 * gl_test_passthrough_vs_src. Declaration order is the twin's location
 * order (a_pos 0, a_color 1). */
attribute vec4 a_pos;
attribute vec4 a_color;
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
}
