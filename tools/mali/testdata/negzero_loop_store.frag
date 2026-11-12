precision highp float; varying vec2 a2; 
void main() { float z = 1.0; float s = 0.0; for (int i = 0; i < 2; i++) { if (a2.x < -z) s += 0.25; z = 0.0; } gl_FragColor = vec4(s, a2.y, 0.0, 1.0); }
