#version 100
precision highp float;
uniform float u_x, u_q, u_a, u_b;
void main() {
  float t = max(u_q, 0.0);
  gl_FragColor = vec4(min(t, 1.0) * 0.5 + 0.25, t * 0.5, clamp(u_a, 0.0, 0.8) * 0.5 + 0.25,
                      min(max(u_x, 0.2), 1.0) * 0.5 + 0.25);
}
