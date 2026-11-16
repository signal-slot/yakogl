#version 100
/* dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec2_xx_vertex,
 * rebuilt from VK-GL-CTS 93bca018 (Apache-2.0): the generated fragment shader,
 * modules/glshared/glsShaderLibraryCase.cpp:233-282 (genFragmentShader) with
 * framework/opengl/gluShaderLibrary.cpp:1912-1991 (genCompareFunctions). */
precision mediump float;
precision mediump int;

bool isOk (vec2 a, vec2 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }

varying vec2 out0;
uniform vec2 ref_out0;

void main()
{
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
