precision mediump float;
varying vec4 dist;
const vec4 LINE = vec4(1.0);
const vec4 FILL = vec4(0.0, 0.5, 0.8, 0.8);
void main(void) {
    float d = min(dist.x * dist.w, min(dist.y * dist.w, dist.z * dist.w));
    float I = exp2(-2.0 * d * d);
    gl_FragColor = mix(FILL, LINE, I);
}
