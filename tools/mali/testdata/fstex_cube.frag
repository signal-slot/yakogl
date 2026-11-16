/* dEQP-GLES2 texture.{filtering,size,completeness,format}.*cube*: the
 * fragment shader of glsTextureTestUtil.cpp ProgramLibrary, PROGRAM_CUBE_FLOAT
 * at mediump, as GLSL ES 1.00 makes it. The Mesa 26.2.1 drm-shim reference. */
varying mediump vec3 v_texCoord;
uniform mediump float u_bias;
uniform mediump float u_ref;
uniform mediump vec4 u_colorScale;
uniform mediump vec4 u_colorBias;
uniform mediump samplerCube u_sampler;

void main (void)
{
    gl_FragColor = textureCube(u_sampler, v_texCoord) * u_colorScale + u_colorBias;
}
