precision mediump float;
varying vec4 x;
void main(void) { gl_FragColor = clamp(x, 0.25, 0.75); }
