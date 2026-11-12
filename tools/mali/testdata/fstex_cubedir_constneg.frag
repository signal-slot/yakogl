precision mediump float; uniform samplerCube s; varying vec2 a2;
void main() {
    gl_FragColor = textureCube(s, vec3(a2.x, -1.0, a2.y));
}
