precision mediump float;
uniform vec4 v[2];
uniform sampler2D t;
uniform samplerCube s;
varying mediump vec2 a2;
void main() { gl_FragColor = textureCube(s, vec3(a2, 1.0)); }
