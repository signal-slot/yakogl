#version 100
precision highp float;
uniform float u_x, u_q, u_a, u_b;
void main() { gl_FragColor = vec4(2.0 * (u_q * 3.0), 2.0 * (u_a * 3.0), 0.0, 1.0); }
