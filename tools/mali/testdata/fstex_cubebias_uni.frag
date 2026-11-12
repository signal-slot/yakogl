precision mediump float; uniform samplerCube s; uniform float u; varying vec3 v3;
void main() {
    gl_FragColor = textureCube(s, v3, u);
}
