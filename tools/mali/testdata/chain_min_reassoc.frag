#version 100
precision highp float;
uniform float u_q, u_b;
void main() { gl_FragColor = vec4(8.0 * u_q * u_b, 0.0, 0.0, 1.0); }
