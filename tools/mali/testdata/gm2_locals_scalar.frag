#version 100
/* FS for gm2_locals_scalar.vert: v out as the red channel. */
precision mediump float;
varying float v;
void main() { gl_FragColor = vec4(v, 0.0, 0.0, 1.0); }
