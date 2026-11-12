/* Pass-through fragment shader for the vsfold_*const references. */
precision mediump float;
varying vec4 v;
void main() { gl_FragColor = v; }
