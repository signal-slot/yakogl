precision mediump float;
varying mediump vec2 a2;
uniform samplerCube s;
uniform float u;
varying mediump vec2 b2;
void main() { gl_FragColor = textureCube(s, vec3(a2.x, b2.y, 1.0), u); }
