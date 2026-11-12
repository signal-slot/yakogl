/* Load width: one lane of a mediump vec2: Mesa loads .v2. */
precision mediump float;
varying vec2 v;
void main() { gl_FragColor = vec4(v.y, 0.5, 0.25, 0.75); }
