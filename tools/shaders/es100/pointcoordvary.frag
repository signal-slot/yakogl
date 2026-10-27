#version 100
/* gl_PointCoord after a varying: gl_test case 105 (pointcoordvary).
 *
 * R = s, G = t across a 16-pixel point sprite, B = the vertex colour's blue
 * (0.5). The varying is the first fragment input and gl_PointCoord the one
 * after it, so its components start at 4, not 0 -- the offset case 83
 * cannot tell apart from none -- and the varying must still arrive whole
 * beside it. */
precision mediump float;
varying vec4 v_color;
void main() {
    gl_FragColor = vec4(gl_PointCoord, v_color.b, 1.0);
}
