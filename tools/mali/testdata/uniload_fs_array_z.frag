precision highp float; uniform vec4 u[2]; varying vec2 a2;
void main() { gl_FragColor = vec4(a2, u[1].z, 1.0); }
