#version 100
/* MVP VS. ES 1.00 twin of gl_test_mvp_vs_src; case 1's vertex stage. */
uniform mat4 u_mvp;
attribute vec4 a_pos;
void main() { gl_Position = u_mvp * a_pos; }
