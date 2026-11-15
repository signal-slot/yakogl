precision mediump float; uniform samplerCube s; varying mediump vec2 a2;
void main() {
    gl_FragColor = textureCube(s, vec3(-a2.yx, 1.0));
}
