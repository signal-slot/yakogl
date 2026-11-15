precision mediump float; varying mediump vec2 a2;
void main() {
    gl_FragColor = vec4(-a2.yx, 0.0, 1.0);
}
