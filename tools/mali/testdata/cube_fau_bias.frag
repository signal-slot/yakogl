precision mediump float; uniform samplerCube s; uniform float u; varying float b1;
void main() {
    gl_FragColor = textureCube(s, vec3(1.0, b1, 0.5), u);
}
