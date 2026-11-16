#version 100
/* FRAGCOORD_FP16: fract of the scaled product, read through a vec2 copy of gl_FragCoord.xy (the head of conditionals, function and loop) */
precision mediump float;
void main() {
    vec2 fc = gl_FragCoord.xy;
    float d = fract(fc.x * fc.y * 0.0001);
    gl_FragColor = vec4(d, d, d, 1.0);
}
