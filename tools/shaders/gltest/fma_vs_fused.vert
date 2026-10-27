#version 100
attribute vec4 a_pos;
uniform float u_x, u_q, u_a, u_b;
varying vec4 v;
void main() { gl_Position = a_pos; v = vec4(u_x - 2.0 * u_q, u_x - u_a * u_b, 2.0 * u_q + u_b, 1.0); }
