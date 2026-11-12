precision mediump float; varying vec2 v; uniform vec4 u; void main() { if (-u.x < 0.5) discard; gl_FragColor = vec4(1.0); }
