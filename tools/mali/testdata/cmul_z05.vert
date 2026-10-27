/* A constant z of 0.5: the viewport transform's 0.5 * scale.z (qt3d ssao_300.vert's position, in ES 1.00). */
attribute vec4 a_pos;
void main() { gl_Position = vec4(a_pos.xy, 0.5, 1.0); }
