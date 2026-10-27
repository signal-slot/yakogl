#version 100
precision highp float;
uniform float u_x, u_q, u_a, u_b;
void main() {
  float p = u_a * u_b;
  gl_FragColor = vec4(u_x - p, p * 2.0, u_x - abs(u_a * u_b), floor(u_a * u_b * 4.0) + 0.25);
}
