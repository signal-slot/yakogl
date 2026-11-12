precision highp float; varying vec2 a2; varying vec2 v; varying vec4 c4; uniform vec4 u;
void main() { bvec2 b = lessThan(a2, vec2(-0.0, -0.0)); gl_FragColor = vec4(b.x ? 1.0 : 0.25, b.y ? 1.0 : 0.5, 0.0, 1.0); }
