precision highp float; varying vec2 a2; varying vec2 v; varying vec4 c4; uniform vec4 u;
void main() { gl_FragColor = vec4(clamp(c4.xyz * -2.0, 0.0, 1.0), 1.0); }
