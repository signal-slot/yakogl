/* dEQP-GLES2.functional.shaders.linkage.varying_4, rebuilt from VK-GL-CTS
 * 93bca018 (Apache-2.0): data/gles2/shaders/linkage.test:63-86 (the vertex
 * source) through modules/glshared/glsShaderLibraryCase.cpp:441-478
 * (generateVertexSpecialization: dEQP_Position and the case's uniform u_false).
 * Shape U of docs/notes/mali-deqp-varylink.md: var is declared, never written. */
attribute highp vec4 dEQP_Position;
uniform bool u_false;

varying mediump float var;
void main()
{
    gl_Position = dEQP_Position;

}
