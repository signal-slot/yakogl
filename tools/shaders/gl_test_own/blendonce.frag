#version 100
precision highp float;
uniform float u_x, u_q, u_a, u_b;
void main() { gl_FragColor = vec4(u_x, u_q, u_a, u_b); }
