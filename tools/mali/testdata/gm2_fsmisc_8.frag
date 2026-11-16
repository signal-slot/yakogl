precision mediump float;
varying vec4 x;
varying vec4 y;
void main(void) { gl_FragColor = step(0.5, x) * y; }
