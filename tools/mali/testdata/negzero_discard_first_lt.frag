precision mediump float;
varying vec2 v;
uniform vec4 u;
void main() {
    if (-0.0 < v.x) discard;
    gl_FragColor = vec4(1.0);
}
