precision highp float; varying vec2 a2; varying vec2 v; 
void main() { gl_FragColor = vec4(a2 * -vec2(1.0, 2.0), 0.0, 1.0); }
