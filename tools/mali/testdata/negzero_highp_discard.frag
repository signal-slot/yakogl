precision highp float; varying vec4 v; uniform vec4 u; void main() { if (v.x < -0.0) discard; gl_FragColor = v; }
