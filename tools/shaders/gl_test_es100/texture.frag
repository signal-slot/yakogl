#version 100
/* Texture-sample FS. ES 1.00 twin of gl_test_texture_fs_src: the twin's
 * descriptor-set sampler becomes a plain `uniform sampler2D`, and `texture()`
 * becomes `texture2D()`, which is the ES 1.00 spelling. */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_tex;
void main() { gl_FragColor = texture2D(u_tex, v_uv); }
