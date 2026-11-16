#version 100
/* ref-texcoord-lane: pass-through position and five varyings (vec3 vd, vec3 va, vec4 vw, vec2 t0, vec2 t1) */
attribute vec4 a_pos;
attribute vec3 a_d;
attribute vec3 a_a;
attribute vec4 a_w;
attribute vec2 a_t0;
attribute vec2 a_t1;
varying vec3 vd;
varying vec3 va;
varying vec4 vw;
varying vec2 t0;
varying vec2 t1;
void main() {
    gl_Position = a_pos;
    vd = a_d;
    va = a_a;
    vw = a_w;
    t0 = a_t0;
    t1 = a_t1;
}
