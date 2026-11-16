/* A mediump float local of a.y read back bare: one lane. */
precision mediump float;
varying vec4 a;
void main() { mediump float t = a.y; gl_FragColor = vec4(1.0); gl_FragColor.x = t; }
