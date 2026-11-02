#version 100
/* Interpolated-colour FS: consumes the passthrough VS's v_color varying.
 * ES 1.00 twin of gl_test_interp_fs_src. */
precision mediump float;
varying vec4 v_color;
void main() { gl_FragColor = v_color; }
