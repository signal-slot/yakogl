#version 300 es
/* Equal lanes, flat p and smooth q: kept apart by Mesa; refused here. */
precision mediump float;
flat in vec2 p; in vec2 q; out vec4 o;
void main() { o = vec4(p.x, q.y, 0.5, 1.0); }
