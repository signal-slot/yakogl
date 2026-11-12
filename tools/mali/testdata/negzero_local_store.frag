precision highp float; varying vec2 a2; varying vec2 v; 
void main() { vec2 z = vec2(-0.0); gl_FragColor = vec4(a2.x < z.x ? 1.0 : 0.5, z, 1.0); }
