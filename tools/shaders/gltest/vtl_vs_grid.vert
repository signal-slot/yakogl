#version 100
attribute vec2 a_pos;
attribute vec2 a_uv;
uniform sampler2D u_vtex;
varying vec4 v_color;
void main() {
    v_color = texture2DLod(u_vtex, a_uv, 0.0);
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
