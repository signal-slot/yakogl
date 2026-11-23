precision mediump float;
varying vec4 a; varying vec4 b; varying vec4 c; varying vec4 d;
void main() { gl_FragColor = (a + b + c + d) * 0.25; }
