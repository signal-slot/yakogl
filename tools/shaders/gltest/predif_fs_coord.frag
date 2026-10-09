#version 100
precision highp float;
uniform float u_x, u_q, u_a, u_b;
void main() {
  vec4 c;
  if (gl_FragCoord.x < 32.0) {
    c = vec4(u_a * 2.0, u_q + 0.5, 0.25, 1.0);
  } else {
    c = vec4(u_q, u_a * 0.5, 0.75, 1.0);
  }
  gl_FragColor = c;
}
