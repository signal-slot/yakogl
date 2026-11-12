precision mediump float; varying vec2 v; void main() { if (0.5 < -v.x) discard; gl_FragColor = vec4(1.0); }
