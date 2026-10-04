#version 100
/* v3d_triangle's discard variant (TRI_DISCARD): the lanes where g > 0.25 and
 * b < 0.25 are discarded, so that part of the triangle keeps the clear
 * colour, and the rest is the interpolated colour. The && combines two
 * conditions, and with these vertex colours each one alone would discard a
 * different region (a threshold of 0.5 would not: g > 0.5 forces b < 0.5). */
precision mediump float;
varying vec4 v_color;
void main() {
    if (v_color.g > 0.25 && v_color.b < 0.25)
        discard;
    gl_FragColor = v_color;
}
