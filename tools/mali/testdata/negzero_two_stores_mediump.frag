precision mediump float; varying vec2 a2; 
void main() { float z = 1.0; z = 0.0; if (a2.x < -z) discard; gl_FragColor = vec4(1.0); }
