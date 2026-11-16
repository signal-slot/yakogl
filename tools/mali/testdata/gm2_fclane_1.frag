#version 100
/* FRAGCOLOR_LANE: lanes taken from other lanes of the varyings (a swizzle across two varyings, no arithmetic) */
precision highp float;
varying vec3 vd;
varying vec3 va;
varying vec4 vw;
varying vec2 t0;
varying vec2 t1;
void main() {
    gl_FragColor = vec4(vd.g, vd.r, vd.b, va.r);
}
