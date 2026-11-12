precision mediump float; varying vec2 v; varying vec2 w; void main() { if (v.x < w.x) discard; gl_FragColor = vec4(1.0); }
