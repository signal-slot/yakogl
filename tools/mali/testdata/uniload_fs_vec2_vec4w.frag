precision highp float; uniform vec2 a; uniform vec4 u; varying vec2 a2;
void main() { gl_FragColor = vec4(a2, u.w, a.x); }
