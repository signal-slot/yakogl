precision mediump float;
varying vec4 v;
varying vec4 w;
void main() {
    gl_FragColor = vec4(v.x, v.y, 0.5, 1.0);
}
