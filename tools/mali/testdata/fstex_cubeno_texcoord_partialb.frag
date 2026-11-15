precision mediump float;
varying mediump vec2 a2;
uniform samplerCube s;
varying mediump vec2 b2;
void main() { gl_FragColor = textureCube(s, vec3(a2, b2.x)); }
