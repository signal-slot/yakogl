#version 100
/* FRAGCOLOR_LANE, jellyfish 8ee5's tail: rgb a sum of varyings times a texel, alpha a texel's alpha plus pow(varying, 2.0) less 0.3 (the coordinates are two whole vec2 varyings) */
precision highp float;
varying vec3 vd;
varying vec3 va;
varying vec4 vw;
varying vec2 t0;
varying vec2 t1;
uniform sampler2D s0;
uniform sampler2D s1;
void main() {
    vec4 c = texture2D(s1, t1);
    vec4 m = texture2D(s0, t0);
    float a = m.a + pow(vd.r, 2.0) - 0.3;
    gl_FragColor = vec4((va + vd + c.rgb) * m.rgb, a);
}
