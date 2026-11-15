precision mediump float; uniform samplerCube s;
void main() {
    gl_FragColor = textureCube(s, vec3(1.0, 0.0, 0.0));
}
