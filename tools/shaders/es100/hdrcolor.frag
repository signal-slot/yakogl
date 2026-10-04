#version 100
/* Case 91: one colour, components above 1.0 included, blended into RGBA16F. */
precision mediump float;
uniform vec4 u_color;
void main() { gl_FragColor = u_color; }
