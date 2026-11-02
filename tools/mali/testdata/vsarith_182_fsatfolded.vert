#version 100
attribute vec4 a_pos;
uniform float u_x, u_q, u_a, u_b;
varying vec4 v;
vec4 h(vec4 c) { return c * 0.5 + 0.25; }
void main() {
  gl_Position = a_pos;
  v = h(vec4(clamp(u_x, 0.0, 1.0), min(max(u_q, 0.0), 1.0), max(min(u_a, 1.0), 0.0),
             min(1.0, max(0.0, u_a - 1.1))));
}
