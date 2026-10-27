#version 100
/* gl_FragCoord read back: gl_test case 82 (fragcentre).
 *
 * R = 4 * floor(x), G = 4 * floor(y), B = 128 * fract(x) + 64 * fract(y):
 * on a 64x64 target the pixel's own coordinates come back in R and G, and B
 * says whether the pixel centre carries the +0.5 (96) or not (0). GL's y
 * counts from the BOTTOM of the window and of an FBO alike; a target drawn
 * upside down (the default framebuffer is stored top-down) shows as
 * G = 4 * (63 - y). */
precision highp float;
void main() {
    vec2 p = floor(gl_FragCoord.xy);
    vec2 f = fract(gl_FragCoord.xy);
    gl_FragColor = vec4(p.x * 4.0 / 255.0, p.y * 4.0 / 255.0, (f.x * 128.0 + f.y * 64.0) / 255.0, 1.0);
}
