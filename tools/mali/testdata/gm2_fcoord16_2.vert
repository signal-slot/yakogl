#version 100
/* ref-fragcoord16: pass-through position only */
attribute vec4 a_pos;
void main() { gl_Position = a_pos; }
