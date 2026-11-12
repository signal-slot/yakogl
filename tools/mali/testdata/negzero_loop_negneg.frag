precision highp float; varying vec2 a2; 
void main() { float z = 0.0; for (int i = 0; i < 1; i++) { z = -z; } gl_FragColor = vec4(a2.x < -z ? 1.0 : 0.25, a2.y, 0.0, 1.0); }
