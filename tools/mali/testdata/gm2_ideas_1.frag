#version 100
/* study-ideas: glmark2 22c527cb ideas, fragment shader df64b720 (the text
 * scene's alpha-tested glyph): the coordinate is gl_FragCoord.xy / 32.0 in a
 * mediump shader (gm2_texcoord_1 has the same body under highp, where
 * the divide is fp32), alpha test with discard. */
precision mediump float;
uniform sampler2D tex;
void main() {
    vec2 p = gl_FragCoord.xy / 32.0;
    vec4 c = texture2D(tex, p);
    if (c.w < 0.5)
        discard;
    gl_FragColor = c;
}
