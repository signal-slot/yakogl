#version 100
/* Pass-through fragment shader for es100/vrcp.vert: draws its v_t varying. */
precision mediump float;
varying vec4 v_t;
void main() { gl_FragColor = v_t; }
