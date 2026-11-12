/* p and q merged (one .v2 load). */
precision mediump float;
varying vec2 p; varying vec2 q;
void main() { gl_FragColor = vec4(p.x, q.y, 0.5, 1.0); }
