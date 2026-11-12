/* A discard of a.x, then the colour's pair z,w converted after the DISCARD. */
precision mediump float;
varying vec4 a;
void main() { if (a.x > 0.5) discard; gl_FragColor = vec4(a.z, a.w, 0.0, 1.0); }
