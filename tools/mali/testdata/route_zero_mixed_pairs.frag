precision mediump float;
varying vec4 v;
varying vec4 w;
void main() {
    gl_FragColor = v + w * vec4(0.0, 0.0, -0.0, -0.0);
}
