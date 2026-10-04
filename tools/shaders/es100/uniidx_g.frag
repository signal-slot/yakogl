/* gl_test 120 uniidxg: 108 D's fcols[fidx] with a texld selected between
 * the movar and the relative read, as gl_test 100 idxtex's fragment stage
 * has it (idxtex passes on the i.MX8MP; D reads element 0). */
precision mediump float;
uniform vec4 fcols[4];
uniform int fidx;
uniform sampler2D ftex;
void main() {
    gl_FragColor = 0.5 * (fcols[fidx] + texture2D(ftex, vec2(0.5)));
}
