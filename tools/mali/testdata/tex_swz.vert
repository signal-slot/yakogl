#version 100
/* tc2.vert of docs/notes/mali-tex-coords.md: a pass-through position and one
 * vec2 varying, the layout of the tex_* coordinate variants with a vec2
 * varying. */
attribute vec4 a_pos;
attribute vec2 a_v;
varying vec2 v;
void main() {
    gl_Position = a_pos;
    v = a_v;
}
