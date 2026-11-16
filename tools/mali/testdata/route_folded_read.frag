precision mediump float;
varying mediump float e;
uniform mediump float u;
void main() {
    gl_FragColor = vec4(e, u, 0.0, 0.0) * vec4(0.0, 0.0, -0.375, u);
}
