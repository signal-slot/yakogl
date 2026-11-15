precision mediump float;
uniform samplerCube sa[2];
varying mediump vec2 a2;
void main() {
    gl_FragColor = textureCube(sa[1], vec3(a2, 1.0));
}
