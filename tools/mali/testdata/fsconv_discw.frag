/* A discard of a.w, then the colour's pair x,y converted after the DISCARD. */
precision mediump float;
varying vec4 a;
void main() { if (a.w > 0.5) discard; gl_FragColor = vec4(a.x, a.y, 0.0, 1.0); }
