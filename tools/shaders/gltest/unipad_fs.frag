#version 100
precision mediump float;
uniform vec2 u_rg;
void main() { gl_FragColor = vec4(u_rg, 0.0, 1.0); }
