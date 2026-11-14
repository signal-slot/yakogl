#version 100
/* FRAGCOLOR_LANE: the tail of fclane_2 with pow(vd.r, 2.0) written vd.r * vd.r */
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
    float a = m.a + vd.r * vd.r - 0.3;
    gl_FragColor = vec4((va + vd + c.rgb) * m.rgb, a);
}
