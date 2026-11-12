precision highp float; varying vec2 a2; void main() { const vec2 z = vec2(-0.0); gl_FragColor = vec4(z.x == a2.x ? 1.0 : 0.25, a2.y == z.y ? 1.0 : 0.5, 0.0, 1.0); }
