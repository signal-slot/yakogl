precision mediump float; uniform vec4 u4; uniform sampler2D t; varying vec2 a2;
void main() {
    gl_FragColor = texture2D(t, a2) * u4.w;
}
