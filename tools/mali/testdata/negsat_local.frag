precision highp float; varying vec2 a2; varying vec2 v; varying vec4 c4; uniform vec4 u;
void main() { vec2 t = a2 * -2.0; gl_FragColor = vec4(clamp(t, 0.0, 1.0), clamp(t, 0.0, 1.0).yx); }
