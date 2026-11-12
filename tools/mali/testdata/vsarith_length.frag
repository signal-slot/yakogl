/* Pass-through fragment shader for vsarith_length.vert: draws its v varying. */
precision mediump float;
varying vec4 v;
void main() { gl_FragColor = v; }
