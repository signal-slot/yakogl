precision mediump float; varying vec2 a2; uniform vec2 u; void main() { gl_FragColor = vec4(a2 * u * -3.0, 0.0, 1.0); }
