precision highp float; varying vec2 a2; 
void main() { const int k = 0; gl_FragColor = vec4(a2.x < -float(k) ? 1.0 : 0.25, a2.y, 0.0, 1.0); }
