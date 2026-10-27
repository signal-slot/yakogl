#version 100
/* Two samplers declared, only the second used: the case that a used non-zero
 * binding still lands where the driver expects. ES 1.00 twin of
 * gl_test_compact_fs_src, and both samplers are declared here for the same
 * reason the twin declares them -- the unused one is the point. */
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_tex0;
uniform sampler2D u_tex1;
void main() { gl_FragColor = texture2D(u_tex1, v_uv); }
