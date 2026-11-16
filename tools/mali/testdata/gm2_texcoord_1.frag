#version 100
/* TEX_COORD, ideas df64's shape: the coordinate is gl_FragCoord.xy / 32.0, alpha test with discard */
precision highp float;
uniform sampler2D tex;
void main() {
    vec2 p = gl_FragCoord.xy / 32.0;
    vec4 c = texture2D(tex, p);
    if (c.w < 0.5)
        discard;
    gl_FragColor = c;
}
