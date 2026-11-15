precision mediump float; uniform samplerCube s; varying vec4 v4;
void main() {
    gl_FragColor = textureCube(s, v4.xyz);
}
