#version 100
/* Integer division on data (v3d back end: nir_lower_idiv's 32-bit sequence).
 * Dividends up to about a million, divisors 1..10 and their negatives, so
 * the quotient's sign rule, truncation toward zero and the reciprocal's
 * refinement steps are all exercised. */
precision highp float;
precision highp int;
varying vec4 v_color;
void main() {
    int i = int(v_color.r * 2000000.0) - 1000000;
    int j = int(v_color.g * 9.0) + 1;
    int k = int(v_color.b * 50.0) - 25;
    int q1 = i / j;
    int q2 = i / (-j);
    int q3 = k / 3;
    gl_FragColor = vec4(float(q1) / 1000000.0, float(q2) / 1000000.0, float(q3) / 25.0, float(i - q1 * j) / 10.0);
}
