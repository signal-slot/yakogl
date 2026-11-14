#version 100
/* ref-texcoord-lane: pass-through position only */
attribute vec4 a_pos;
void main() { gl_Position = a_pos; }
