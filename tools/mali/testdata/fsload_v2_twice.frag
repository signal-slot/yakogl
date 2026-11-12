/* Load width: one lane of a vec2 read twice: Mesa loads .v2. */
precision mediump float;
varying vec2 v;
void main() { gl_FragColor = vec4(v.x, 0.5, v.x, 0.75); }
