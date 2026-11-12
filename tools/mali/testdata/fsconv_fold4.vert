/* A vec4 and a vec2 varying (the vertex shader of fsconv_fold4). */
attribute vec4 a_pos; attribute vec4 a_a; attribute vec4 a_b; varying vec4 a; varying vec2 t; void main() { gl_Position = a_pos; a = a_a; t = a_b.xy; }
