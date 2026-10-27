/* The vertex shader of the board reference program tri_varying.c (Mesa 25.0.7 on
 * the NanoPC-T6, tools/mali/README-pan_shc.md), verbatim. */
attribute vec2 pos; attribute vec3 col; varying vec3 vcol; void main(){ vcol = col; gl_Position = vec4(pos, 0.0, 1.0); }
