precision mediump float; uniform samplerCube s; varying mediump vec2 a2; varying mediump vec2 b2;
void main() {
    gl_FragColor = textureCube(s, vec3(a2 * 2.0 + b2 * 5.0, 1.0));
}
