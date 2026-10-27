/* gl_test 108 uniidx, program D: gl_test 100 idxtex's fragment shape
 * (vec4 fcols[4] at a uniform int, which passes on the i.MX8MP) in
 * ES 1.00, without the texture. */
precision mediump float;
uniform vec4 fcols[4];
uniform int fidx;
void main() {
    gl_FragColor = fcols[fidx];
}
