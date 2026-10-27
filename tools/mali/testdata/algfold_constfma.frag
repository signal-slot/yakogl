#version 100
precision highp float;
uniform float u_x, u_q, u_a, u_b;
void main() { gl_FragColor = vec4(dot(vec2(u_q, u_a), vec2(1.0, 2.0)), dot(vec2(u_x, u_b), vec2(-1.0, 3.0)), 0.0, 1.0); }
