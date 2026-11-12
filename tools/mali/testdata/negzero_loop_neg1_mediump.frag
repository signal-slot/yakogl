precision mediump float; varying vec2 a2; 
void main() { float z = 0.0; for (int i = 0; i < 1; i++) { z = -z; } if (a2.x < z) discard; gl_FragColor = vec4(1.0); }
