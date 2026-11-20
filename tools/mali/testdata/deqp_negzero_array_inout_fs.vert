#version 100
/* dEQP-GLES2.functional.shaders.functions.array_arguments.copy_local_inout_on_call_fragment:
 * the generated vertex shader (glsShaderLibraryCase.cpp genVertexShader). */
precision highp float;
precision highp int;

attribute highp vec4 dEQP_Position;
attribute vec4 a_in0;
varying vec4 in0;

void main()
{
    gl_Position = dEQP_Position;
    in0 = a_in0;
}
