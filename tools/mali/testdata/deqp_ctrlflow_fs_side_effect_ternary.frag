precision mediump float;
bool correct = true;
uniform float u_zero;
float wrong() {
    correct = false;
    return 0.0;
}
void main() {
    float a = u_zero - 0.5; // Result should be -0.5.
    float green = (a++, a > 0.0 ? 1.0 : wrong());
    gl_FragColor = vec4(0.0, correct ? green : 0.0, 0.0, 1.0);
}
