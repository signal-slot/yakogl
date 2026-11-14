#version 100
/* glmark2 desktop.vert / effect-2d.vert (a position and a texcoord attribute,
 * one varying vec2) as the vertex shader of the gm2_constfold_* reductions. */
attribute vec2 position;
attribute vec2 texcoord;
varying vec2 TextureCoord;
void main(void) {
    gl_Position = vec4(position, 0.0, 1.0);
    TextureCoord = texcoord;
}
