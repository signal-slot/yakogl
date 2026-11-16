#version 300 es
/* q.x is p.y, q flat: kept apart by Mesa; refused here. */
precision mediump float;
in vec2 p; flat in vec2 q; out vec4 o;
void main() { o = vec4(p.x, q.x, 0.5, 1.0); }
