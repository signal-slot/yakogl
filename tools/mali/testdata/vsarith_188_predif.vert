#version 100
attribute vec4 a_pos;
uniform float u_x, u_q, u_a, u_b;
varying vec4 v;
void main() {
  gl_Position = a_pos;
  if (u_x > 0.0) {
    v = vec4(u_a * 2.0, u_q + 0.5, 0.25, 1.0);
  } else {
    v = vec4(u_q, u_a * 0.5, 0.75, 1.0);
  }
}
