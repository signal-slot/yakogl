// SPDX-License-Identifier: Apache-2.0
// Generated: see README, 'Stepping the temps axis'.
attribute vec4 a_position;
varying vec4 v_color;
void main() {
  vec4 p = a_position;
  vec4 t0 = p + vec4(0.000);
  vec4 t1 = p + vec4(0.001);
  vec4 t2 = p + vec4(0.002);
  vec4 t3 = p + vec4(0.003);
  vec4 t4 = p + vec4(0.004);
  vec4 t5 = p + vec4(0.005);
  vec4 t6 = p + vec4(0.006);
  vec4 t7 = p + vec4(0.007);
  vec4 a = t0 + t1 + t2 + t3 + t4 + t5 + t6 + t7;
  vec4 b = t0 * t1 * t2 * t3 * t4 * t5 * t6 * t7;
  gl_Position = a;
  v_color = b;
}
