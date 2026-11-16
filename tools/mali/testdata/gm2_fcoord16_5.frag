#version 100
/* FRAGCOORD_FP16: the head, then a branch on the value (conditionals shape) */
precision mediump float;
void main() {
    vec2 fc = gl_FragCoord.xy;
    float d = fract(fc.x * fc.y * 0.0001);
    if (d > 0.5) {
        d = d - 0.5;
    } else {
        d = d + 0.25;
    }
    gl_FragColor = vec4(d, d, d, 1.0);
}
