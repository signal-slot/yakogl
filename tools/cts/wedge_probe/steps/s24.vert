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
  vec4 t8 = p + vec4(0.008);
  vec4 t9 = p + vec4(0.009);
  vec4 t10 = p + vec4(0.010);
  vec4 t11 = p + vec4(0.011);
  vec4 t12 = p + vec4(0.012);
  vec4 t13 = p + vec4(0.013);
  vec4 t14 = p + vec4(0.014);
  vec4 t15 = p + vec4(0.015);
  vec4 t16 = p + vec4(0.016);
  vec4 t17 = p + vec4(0.017);
  vec4 t18 = p + vec4(0.018);
  vec4 t19 = p + vec4(0.019);
  vec4 t20 = p + vec4(0.020);
  vec4 t21 = p + vec4(0.021);
  vec4 t22 = p + vec4(0.022);
  vec4 t23 = p + vec4(0.023);
  vec4 a = t0 + t1 + t2 + t3 + t4 + t5 + t6 + t7 + t8 + t9 + t10 + t11 + t12 + t13 + t14 + t15 + t16 + t17 + t18 + t19 + t20 + t21 + t22 + t23;
  vec4 b = t0 * t1 * t2 * t3 * t4 * t5 * t6 * t7 * t8 * t9 * t10 * t11 * t12 * t13 * t14 * t15 * t16 * t17 * t18 * t19 * t20 * t21 * t22 * t23;
  gl_Position = a;
  v_color = b;
}
