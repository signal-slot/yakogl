precision highp float; varying vec2 a2; 
void main() { gl_FragColor = vec4(clamp(a2 * -2.0, 0.0, 1.0), 0.0, 1.0); }
