precision highp float; varying vec2 a2; void main() { float k = 1.0e-30; float d = k * 1.0e-15; gl_FragColor = vec4(a2.x < d ? 1.0 : 0.25, a2.y, 0.0, 1.0); }
