#version 100
/* glmark2 22c527cb terrain, fragment shader 1637be2e (the noise pass), its
 * call shape only: a float function of a vec3 that calls two vec4 helper
 * functions (one with a highp parameter), called four times with scaled
 * arguments by a second function whose result is the colour. No branch;
 * the noise arithmetic itself is left out (gm2_locals_* and the ARITH
 * cases hold it). */
precision mediump float;
uniform float u_time;
varying vec2 v_uv;
vec4 shuffle(highp vec4 x) {
    return (x * 5.0 + 1.0) * x;
}
vec4 scale(vec4 r) {
    return 1.5 - 0.75 * r;
}
float bump(vec3 c) {
    vec4 q = shuffle(vec4(c, 1.0));
    q = scale(q * q);
    return dot(q, vec4(0.25));
}
float layers(vec3 c) {
    float n = 0.0;
    n += 1.0 * abs(bump(c));
    n += 0.5 * abs(bump(c * 2.0));
    n += 0.25 * abs(bump(c * 4.0));
    n += 0.125 * abs(bump(c * 8.0));
    return n;
}
void main() {
    float n = layers(vec3(v_uv, -u_time));
    gl_FragColor = vec4(vec3(n), 1.0);
}
