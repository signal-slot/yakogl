#version 100
precision highp float;
uniform float u_x, u_q, u_a, u_b;
void main() {
  gl_FragColor = vec4(abs(u_x - 2.0 * u_a) * 1.0, 1.0 * -(u_q - u_a),
                      (u_b * 1.0) * u_q * 8.0, 1.0);
}
