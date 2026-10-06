#version 100
precision highp float;
uniform float u_x, u_q, u_a, u_b;
void main() {
  float a = u_q * 3.0;
  float b = u_a + 0.125;
  float r, s;
  if (gl_FragCoord.x < 32.0) { r = a * 2.0; s = b - a; } else { r = b * 0.5; s = a + b; }
  gl_FragColor = vec4(r, s, a, b);
}
