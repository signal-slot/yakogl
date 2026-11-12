precision highp float; varying vec2 a2; void main() { if (a2.x == vec2(-0.0).x) discard; if (vec2(-0.0).y == a2.y) discard; gl_FragColor = vec4(1.0); }
