/* gl_FragColor.x = a.y * 1.0: one lane. */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(0.5); gl_FragColor.x = a.y * 1.0; }
