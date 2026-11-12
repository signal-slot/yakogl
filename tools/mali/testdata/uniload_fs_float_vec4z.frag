precision highp float; uniform float f; uniform vec4 u; varying vec2 a2;
void main() { gl_FragColor = vec4(a2, u.z, f); }
