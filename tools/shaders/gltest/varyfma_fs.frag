#version 100
precision highp float;
varying vec4 v;
void main() { gl_FragColor = vec4(v.x * 2.0 - 1.0, v.y * 0.5 + 0.25, 1.0 - v.x * v.y, 1.0); }
