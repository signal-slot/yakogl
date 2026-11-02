#version 100
attribute vec4 a_pos;
uniform float u_x, u_q, u_a, u_b;
varying vec4 v;
void main() {
  gl_Position = a_pos;
  float t = max(u_q, 0.0);
  v = vec4(min(t, 1.0) * 0.5 + 0.25, t * 0.5, clamp(u_a, 0.0, 0.8) * 0.5 + 0.25,
           min(max(u_x, 0.2), 1.0) * 0.5 + 0.25);
}
