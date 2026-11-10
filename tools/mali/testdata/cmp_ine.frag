/* ine_hp of docs/notes/mali-compare-forms.md: float(m != 3) at highp,
 * ICMP_OR.u32.ne.m1 with the constant 3 as the .b3 select immediate.
 * Drawn without --uniform-values, so the int uniform stays 0. */
precision highp float;
precision highp int;
uniform highp int m;
varying vec4 v;
void main() {
    gl_FragColor = vec4(float(m != 3), 0.0, 0.0, 1.0);
}
