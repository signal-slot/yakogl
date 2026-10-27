/* gl_test 122 uniidxi: 108 D's fcols[fidx] with the texld selected before
 * the movar; with 120 and 121 it says where a texld must sit. */
precision mediump float;
uniform vec4 fcols[4];
uniform int fidx;
uniform sampler2D ftex;
void main() {
    vec4 t = texture2D(ftex, vec2(0.5));
    gl_FragColor = 0.5 * (fcols[fidx + int(t.w * 0.0)] + t);
}
