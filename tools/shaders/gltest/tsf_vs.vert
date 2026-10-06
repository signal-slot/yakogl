#version 100
attribute vec4 a_pos;
varying vec2 v_uv;
void main() { v_uv = a_pos.xy * 0.5 + 0.5; gl_Position = a_pos; }
