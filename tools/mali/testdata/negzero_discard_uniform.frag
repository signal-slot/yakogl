precision mediump float; varying vec4 v; uniform vec4 u; void main() { if (u.x < -0.0) discard; gl_FragColor = v; }
