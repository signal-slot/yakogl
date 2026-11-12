precision highp float; varying vec2 v; void main() { gl_FragColor = vec4(-v.x < 0.5 ? 1.0 : 0.25, v.y, 0.0, 1.0); }
