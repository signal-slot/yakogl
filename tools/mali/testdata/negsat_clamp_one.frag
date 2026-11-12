precision highp float; varying vec2 a2; varying vec2 v; uniform vec4 u;
void main() { gl_FragColor = vec4(clamp(a2 * -1.0, 0.0, 1.0), 0.0, 1.0); }
