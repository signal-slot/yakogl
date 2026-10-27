#version 100
precision highp float;
uniform float u_x;
void main() { gl_FragColor = vec4(u_x * 1.0, 0.0, 0.0, 1.0); }
