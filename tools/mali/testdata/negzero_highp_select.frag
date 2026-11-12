precision highp float; varying vec4 v; uniform vec4 u; void main() { gl_FragColor = vec4(v.x < -0.0 ? 1.0 : 0.5, v.yz, 1.0); }
