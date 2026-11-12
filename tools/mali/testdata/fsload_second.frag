/* Load width: one lane of each of two vec4s: two .v4 loads. */
precision mediump float;
varying vec4 a;
varying vec4 b;
void main() { gl_FragColor = vec4(a.x, b.y, 0.25, 1.0); }
