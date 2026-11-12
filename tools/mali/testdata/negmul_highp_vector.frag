precision highp float; varying vec2 a2; 
void main() { gl_FragColor = vec4(a2 * -vec2(2.0, 3.0), 0.0, 1.0); }
