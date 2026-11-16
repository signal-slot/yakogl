#version 100
/* FRAGCOORD_FP16: the head, then a function call and a counted loop (function and loop shape) */
precision mediump float;
float process(float d) {
    return d * 0.5 + 0.1;
}
void main() {
    vec2 fc = gl_FragCoord.xy;
    float d = fract(fc.x * fc.y * 0.0001);
    d = process(d);
    for (int i = 0; i < 4; i++) {
        d = fract(d * 1.7);
    }
    gl_FragColor = vec4(d, d, d, 1.0);
}
