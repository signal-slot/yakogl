precision mediump float; uniform samplerCube s; varying vec2 a2; varying float b1;
void main() {
    gl_FragColor = textureCube(s, vec3(a2, b1));
}
