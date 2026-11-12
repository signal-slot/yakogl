#version 300 es
/* Equal lanes of two interpolation qualifiers: Mesa keeps them apart (q flat, its own load); refused here, flat beside smooth. */
precision mediump float;
in vec2 p; flat in vec2 q; out vec4 o;
void main() { o = vec4(p.x, q.y, 0.5, 1.0); }
