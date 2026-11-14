precision mediump float;
varying vec4 a;
varying vec4 b;
varying float t;
void main(void) { gl_FragColor = mix(a, b, t); }
