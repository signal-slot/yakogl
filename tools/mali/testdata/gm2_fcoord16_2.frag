#version 100
/* FRAGCOORD_FP16: the product scaled by a constant, mediump */
precision mediump float;
void main() {
    float d = gl_FragCoord.x * gl_FragCoord.y * 0.0001;
    gl_FragColor = vec4(d, d, d, 1.0);
}
