precision mediump float;
varying vec4 v;
varying vec4 w;
invariant gl_FragColor;
void main() {
    gl_FragColor = v + w * 0.0;
}
