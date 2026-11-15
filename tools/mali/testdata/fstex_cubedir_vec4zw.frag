precision mediump float; uniform samplerCube s; uniform vec4 u4; varying mediump vec2 a2;
void main() {
    gl_FragColor = textureCube(s, vec3(a2 + u4.zw, 1.0));
}
