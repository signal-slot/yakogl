#version 100
precision mediump float;
uniform sampler2D u_ftex;
uniform float u_pick;
varying vec4 v_color;
void main() { gl_FragColor = mix(v_color, texture2D(u_ftex, vec2(0.5, 0.5)), u_pick); }
