/* ine0_hp of docs/notes/mali-compare-forms.md: float(m != 0) at highp, the
 * compare against the constant zero with no leading MOV. Drawn without
 * --uniform-values, so the int uniform stays 0. */
precision highp float;
precision highp int;
uniform highp int m;
varying vec4 v;
void main() {
    gl_FragColor = vec4(float(m != 0), 0.0, 0.0, 1.0);
}
