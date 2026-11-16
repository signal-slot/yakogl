precision mediump float;
varying vec4 v;
varying vec4 w;
void main() {
    float unused = w.x * 2.0;
    gl_FragColor = v;
}
