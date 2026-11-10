#version 100
/* tc.vert of docs/notes/mali-tex-coords.md: a pass-through position and one
 * vec4 varying, the layout of the tex_* coordinate variants that read a
 * vec4 varying as a coordinate. */
attribute vec4 a_pos;
attribute vec4 a_v;
varying vec4 v;
void main() {
    gl_Position = a_pos;
    v = a_v;
}
