/* ineif_hp of docs/notes/mali-compare-forms.md: if (m != 0) c.rgb *= 0.5 at
 * highp: the if is flattened (no branch), one compare and one CSEL.u32.eq
 * per lane. Drawn without --uniform-values, so the int uniform stays 0. */
precision highp float;
precision highp int;
uniform highp int m;
varying vec4 v;
void main() {
    vec4 c = v;
    if (m != 0) c.rgb *= 0.5;
    gl_FragColor = c;
}
