#version 100
/* dEQP-GLES2.functional.shaders.functions.array_arguments.copy_local_inout_on_call_vertex:
 * the generated fragment shader (glsShaderLibraryCase.cpp genFragmentShader). */
precision mediump float;
precision mediump int;

bool isOk (vec4 a, vec4 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }

varying vec4 out0;
uniform vec4 ref_out0;

void main()
{
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
