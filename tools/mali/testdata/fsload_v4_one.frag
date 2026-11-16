/* Load width: one lane of a mediump vec4: Mesa loads .v4. */
precision mediump float;
varying vec4 a;
void main() { gl_FragColor = vec4(a.y, 0.5, 0.25, 1.0); }
