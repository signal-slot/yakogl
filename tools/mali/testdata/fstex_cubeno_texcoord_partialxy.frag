precision mediump float;
uniform samplerCube s;
varying mediump vec3 a3;
void main() { gl_FragColor = textureCube(s, vec3(a3.xy, 1.0)); }
