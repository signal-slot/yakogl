#pragma STDGL invariant(all)
precision mediump float;
varying vec4 v;
varying vec4 w;
void main() { gl_FragColor = v + w * 0.0; }
