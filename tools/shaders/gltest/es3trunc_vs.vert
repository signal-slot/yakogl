#version 300 es
in vec4 a_pos;
uniform float u_x, u_q, u_a, u_b;
out vec4 v;
void main() { gl_Position = a_pos; v = vec4(u_x, u_q, u_a, u_b); }
