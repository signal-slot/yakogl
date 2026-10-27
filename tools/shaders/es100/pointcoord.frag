#version 100
/* gl_PointCoord read back: gl_test case 83 (pointcoord).
 *
 * R = s, G = t across a 16-pixel point sprite. The vertex shader (points.vert)
 * also passes a constant colour varying, which this shader does not read: a
 * point coordinate taken from the wrong coefficients reads that colour
 * instead of a ramp. */
precision mediump float;
void main() {
    gl_FragColor = vec4(gl_PointCoord, 0.0, 1.0);
}
