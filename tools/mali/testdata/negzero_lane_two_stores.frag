precision highp float; varying vec2 a2; 
void main() { vec2 z = vec2(1.0); z.y = 0.0; gl_FragColor = vec4(a2.x < -z.y ? 1.0 : 0.25, a2.y, 0.0, 1.0); }
