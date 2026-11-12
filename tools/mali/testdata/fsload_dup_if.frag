/* p and q are equal on both sides of the vertex shader's if: merged (one .v2 load, two stores). */
precision mediump float;
varying vec2 p; varying vec2 q;
void main() { gl_FragColor = vec4(p.x, q.y, 0.5, 1.0); }
