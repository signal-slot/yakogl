precision mediump float;
uniform samplerCube sa[2];
uniform samplerCube s;
varying mediump vec2 a2;
void main() {
    gl_FragColor = textureCube(s, vec3(a2, 1.0));
}
