/* dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec2_xx_vertex,
 * rebuilt from VK-GL-CTS 93bca018 (Apache-2.0): data/gles2/shaders/swizzles.test:27-46
 * (the "both" source) through modules/glshared/glsShaderLibraryCase.cpp:285-356
 * (specializeVertexShader) and framework/opengl/gluShaderLibrary.cpp:473-557
 * (the indentation removal). Shape E1 of docs/notes/mali-deqp-varylink.md:
 * out0's two lanes hold the same attribute lane. */
precision mediump float;

attribute highp vec4 dEQP_Position;
attribute vec2 in0;
varying vec2 out0;


void main()
{

    out0 = in0.xx;
    gl_Position = dEQP_Position;

}
