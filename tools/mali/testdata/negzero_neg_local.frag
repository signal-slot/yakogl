precision highp float; varying vec2 a2; varying vec2 v; varying vec4 c4; uniform vec4 u;
void main() { float z = 0.0; gl_FragColor = vec4(a2.x < -z ? 1.0 : 0.5, a2.y, 0.0, 1.0); }
