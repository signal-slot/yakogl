precision mediump float;
varying vec4 v;
varying vec4 w;
void main() {
    if ((vec4(w.x, 2.0, 1.0, 1.0) * vec4(2.0, 3.0, 1.0, 1.0)).y > 6.5) discard;
    gl_FragColor = v;
}
