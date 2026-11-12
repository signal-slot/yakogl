/* Mesa's link merges q into p (q.x is p.y, q.y is p.x): one .v2 load, one pair conversion. */
precision mediump float;
varying vec2 p;
varying vec2 q;
void main() { gl_FragColor = vec4(p.x, q.x, 0.5, 1.0); }
