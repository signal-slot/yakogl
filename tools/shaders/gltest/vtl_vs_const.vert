#version 100
attribute vec2 a_uv;
uniform sampler2D u_vtex;
varying vec4 v_color;
void main() {
    v_color = texture2DLod(u_vtex, vec2(0.53125, 0.53125), 0.0);
    gl_Position = vec4(a_uv * 2.0 - 1.0 + vec2(1.0 / 64.0), 0.0, 1.0);
    gl_PointSize = 1.0;
}
