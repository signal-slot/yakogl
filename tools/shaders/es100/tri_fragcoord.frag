#version 100
/* v3d_triangle's gl_FragCoord variant (TRI_FRAGCOORD). Red is the window x
 * over 1280 and green the window y over 512. GL's window y grows UPWARD, so
 * with the flip right green grows toward the top of the picture, and with it
 * wrong it grows toward the bottom: the flip is visible, and tri_check counts
 * pixels of both kinds. Blue stays the interpolated blue, so the triangle's
 * own interpolation is checked too. */
precision mediump float;
varying vec4 v_color;
void main() {
    gl_FragColor = vec4(gl_FragCoord.x / 1280.0, gl_FragCoord.y / 512.0, v_color.b, 1.0);
}
