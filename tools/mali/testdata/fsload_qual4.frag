#version 300 es
/* Equal lanes of one qualifier, flat: merged (one flat load). */
precision mediump float;
flat in vec2 p; flat in vec2 q; out vec4 o;
void main() { o = vec4(p.x, q.y, 0.5, 1.0); }
