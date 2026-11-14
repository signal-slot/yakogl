#version 100
/* passthrough FS for gm2_locals_vsum.vert (no varyings). */
precision mediump float;
uniform vec4 c;
void main() { gl_FragColor = c; }
