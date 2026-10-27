/* gl_test 121 uniidxh: 108 D's fcols[fidx] with the texld selected after
 * the relative read; the control for 120 (uniidx_g.frag). */
precision mediump float;
uniform vec4 fcols[4];
uniform int fidx;
uniform sampler2D ftex;
void main() {
    vec4 c = fcols[fidx];
    gl_FragColor = 0.5 * (c + texture2D(ftex, vec2(c.w * 0.5)));
}
