precision mediump float;
varying vec4 x;
void main(void) {
    float d = fract(3.0 * x.x) * fract(3.0 * x.x);
    gl_FragColor = vec4(d, x.yzw);
}
