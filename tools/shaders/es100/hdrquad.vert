#version 100
/* Case 91: a quad with texture coordinates, for both of its programs. */
attribute vec4 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    gl_Position = a_pos;
    v_uv = a_uv;
}
