#version 100
/* Look-ups inside uniform branches read back: gl_test case "texbranch".
 *
 * Red: a two-armed `if` on u.x whose arms sample different textures (ta's
 * red or tb's red). Green: a one-armed `if` on u.y with a look-up (ta's
 * green, or 0 when the `if` is not taken). Blue: femtovg filter_image's
 * loop shape, a counted loop of 24 passes that breaks on a uniform bound
 * (u.z) and takes two look-ups per pass, each adding 1/48 of tb's blue; the
 * look-ups' coordinates move by u.w per pass (0 in the case: ta and tb are
 * one texel each, so every coordinate reads that texel). On the V3D each
 * `if` is a uniform branch, the look-ups in an untaken arm are not
 * requested, and the last thread switch comes after the branches. Alpha is
 * 1 (a window may keep no alpha). */
precision mediump float;
uniform sampler2D ta;
uniform sampler2D tb;
uniform vec4 u;
varying vec2 v;
void main() {
    vec3 c = vec3(0.0);
    if (u.x > 0.5) {
        c.r = texture2D(ta, vec2(0.5)).r;
    } else {
        c.r = texture2D(tb, vec2(0.5)).r;
    }
    if (u.y > 0.5) {
        c.g = texture2D(ta, vec2(0.5)).g;
    }
    float s = 0.0;
    for (float i = 1.0; i <= 24.0; i += 1.0) {
        if (i >= u.z) {
            break;
        }
        s += texture2D(tb, vec2(0.5) - i * vec2(u.w)).b;
        s += texture2D(tb, vec2(0.5) + i * vec2(u.w)).b;
    }
    c.b = s / 48.0;
    gl_FragColor = vec4(c, 1.0);
}
