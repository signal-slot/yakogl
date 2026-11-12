precision highp float; uniform vec3 u; varying vec2 a2;
void main() { gl_FragColor = vec4(a2, u.y, 1.0); }
