#version 100
/* Signed int orderings on data (v3d back end: Ilt, Ile as MIN.pushc, after
 * Mesa's ntq_emit_comparison). i and j take 0..3 from the interpolated red
 * and green, so each ordering is both true and false across a draw, and
 * the i <= 1 channel sees a constant on the right. */
precision mediump float;
varying vec4 v_color;
void main() {
    int i = int(3.0 * v_color.r);
    int j = int(3.0 * v_color.g);
    gl_FragColor = vec4(i < j ? 1.0 : 0.0, i <= j ? 1.0 : 0.0, j < i ? 1.0 : 0.0, i <= 1 ? 1.0 : 0.25);
}
