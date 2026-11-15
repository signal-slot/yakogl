precision mediump float;
varying mediump vec2 a2;
uniform samplerCube sa[2];
uniform float u;
void main() { gl_FragColor = textureCube(sa[1], vec3(a2, 1.0), u); }
