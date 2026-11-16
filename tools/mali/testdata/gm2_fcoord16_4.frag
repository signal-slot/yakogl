#version 100
/* FRAGCOORD_FP16: the full head expression written on gl_FragCoord directly */
precision mediump float;
void main() {
    float d = fract(gl_FragCoord.x * gl_FragCoord.y * 0.0001);
    gl_FragColor = vec4(d, d, d, 1.0);
}
