precision mediump float; uniform samplerCube s; varying vec2 a2;
void main() {
    gl_FragColor = textureCube(s, vec3(a2 * 3.0 + 0.25, -1.0));
}
