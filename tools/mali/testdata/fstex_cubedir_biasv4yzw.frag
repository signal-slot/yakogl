precision mediump float; uniform samplerCube s; uniform float u; varying mediump vec4 v4;
void main() {
    gl_FragColor = textureCube(s, v4.yzw, u);
}
