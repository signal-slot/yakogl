precision mediump float; varying vec2 v; void main() { if (v.y < v.x) discard; gl_FragColor = vec4(1.0); }
