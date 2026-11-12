precision mediump float; uniform samplerCube s; uniform float u; varying vec2 a2;
void main() {
    gl_FragColor = textureCube(s, vec3(a2 * u, 1.0));
}
