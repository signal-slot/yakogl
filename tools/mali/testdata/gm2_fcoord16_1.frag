#version 100
/* FRAGCOORD_FP16: one product of gl_FragCoord.x and .y, mediump */
precision mediump float;
void main() {
    float d = gl_FragCoord.x * gl_FragCoord.y;
    gl_FragColor = vec4(d, d, d, 1.0);
}
