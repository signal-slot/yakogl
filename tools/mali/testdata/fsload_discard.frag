/* Load width: a discard's compare of one lane: Mesa loads .v4. */
precision mediump float;
varying vec4 a;
void main() { if (a.y < 0.5) discard; gl_FragColor = vec4(0.5, 0.5, 0.25, 1.0); }
