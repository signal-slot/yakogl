#version 100
precision mediump float;
uniform sampler2D u_s;
varying vec2 v_uv;
void main() { gl_FragColor = texture2D(u_s, v_uv); }
