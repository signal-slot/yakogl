#version 100
/* gl_FragCoord read back: gl_test case 119 (pbufup) draws case 82's shader
 * (prec_82.frag) into an EGL pbuffer; the program is the same, only the
 * target (stored y-up, cull state) differs. */
precision highp float;
void main() {
    vec2 p = floor(gl_FragCoord.xy);
    vec2 f = fract(gl_FragCoord.xy);
    gl_FragColor = vec4(p.x * 4.0 / 255.0, p.y * 4.0 / 255.0, (f.x * 128.0 + f.y * 64.0) / 255.0, 1.0);
}
