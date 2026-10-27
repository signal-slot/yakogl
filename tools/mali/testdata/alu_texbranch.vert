#version 100
/* Derivatives read back: gl_test case "deriv" (deriv.frag).
 *
 * The quad covers the 64x64 target and v is its NDC position, so each pixel
 * steps v by 2/64 = 1/32 in x and in y (GL's y, running up). */
attribute vec4 a_pos;
varying vec2 v;
void main() {
    v = a_pos.xy;
    gl_Position = a_pos;
}
