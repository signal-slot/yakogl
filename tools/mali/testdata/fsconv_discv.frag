/* A discard of a.y, then the colour's z after the DISCARD; its y is the compare's conversion. */
precision mediump float;
varying vec4 a;
void main() { if (a.y > 0.5) discard; gl_FragColor = vec4(a.z, a.y, 0.0, 1.0); }
