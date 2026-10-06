#version 100
attribute vec4 a_pos;
uniform float u_x, u_q, u_a, u_b;
varying vec4 v;
void main() {
  gl_Position = a_pos;
  float p = u_a * u_b;
  v = vec4(u_x - p, p * 2.0, u_x - abs(u_a * u_b), floor(u_a * u_b * 4.0) + 0.25);
}
