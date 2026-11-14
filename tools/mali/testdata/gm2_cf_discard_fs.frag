#version 100
/* glmark2 22c527cb ideas, fragment shader df64b720 (the logo's shadow): a
 * texture look-up at gl_FragCoord.xy over a constant, a one-armed `if` on
 * its alpha compared with a constant that discards, then the texel as the
 * colour. */
precision mediump float;
uniform sampler2D u_tex;
void main() {
    vec4 c = texture2D(u_tex, gl_FragCoord.xy / 16.0);
    if (c.a < 0.5)
        discard;
    gl_FragColor = c;
}
