#version 100
/* Vertex shader of the gm2_nonfin_* reductions: a position and a normal
 * attribute, the normal as the one varying vec3 the fragment shaders read. */
attribute vec3 position;
attribute vec3 normal;
varying vec3 Normal;
void main(void) {
    Normal = normal;
    gl_Position = vec4(position, 1.0);
}
