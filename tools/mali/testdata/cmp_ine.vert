#version 100
/* tc.vert of docs/notes/mali-compare-forms.md: a pass-through position and
 * one vec4 varying, the vertex shader of the compare-form reference shaders. */
attribute vec4 a_pos;
attribute vec4 a_v;
varying vec4 v;
void main() {
    gl_Position = a_pos;
    v = a_v;
}
