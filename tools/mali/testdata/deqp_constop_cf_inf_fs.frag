precision highp float; varying vec2 a2; void main() { float k = 3.0e38; float inf = k * 10.0; gl_FragColor = vec4(a2.x < inf ? 1.0 : 0.25, a2.y, 0.0, 1.0); }
