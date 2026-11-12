precision mediump float; uniform vec4 u4; varying vec2 a2;
void main() {
    gl_FragColor = vec4(a2 + u4.zw, 0.0, 1.0);
}
