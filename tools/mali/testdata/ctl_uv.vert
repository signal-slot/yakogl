/* Control: a texture coordinate varying, for the fragment controls. */
attribute vec4 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() { v_uv = a_uv; gl_Position = a_pos; }
