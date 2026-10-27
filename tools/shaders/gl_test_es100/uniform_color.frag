#version 100
/* Uniform/constant colour FS. ES 1.00 twin of gl_test_uniform_fs_src: the
 * twin's push-constant block becomes a plain uniform, which is what an ES 2.0
 * application writes and what glGetUniformLocation("u_color") finds. */
precision mediump float;
uniform vec4 u_color;
void main() { gl_FragColor = u_color; }
