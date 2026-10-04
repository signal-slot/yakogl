/* gl_test 128 uniidxj: 108 D's fcols[fidx] in a fragment stage that also
 * reads a varying, as gl_test 100 idxtex's does (red and green from the
 * vertex stage, blue and alpha from the indexed read). Every program that
 * reads element 0 on the i.MX8MP (108, 116, 117, 120-122) has a fragment
 * stage with no varying input; idxtex, which passes, has one. */
precision mediump float;
uniform vec4 fcols[4];
uniform int fidx;
varying vec4 v_color;
void main() {
    gl_FragColor = vec4(v_color.rg, fcols[fidx].ba);
}
