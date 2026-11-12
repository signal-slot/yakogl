/* dEQP-GLES2 texture.specification.*cube*: es2fTextureSpecificationTests.cpp
 * TexCubeShader for face 1, vec3(+1, -y, -x). The Mesa 26.2.1 drm-shim reference. */
uniform samplerCube u_sampler0;
varying mediump vec2 v_coord;
void main (void)
{
    mediump float x = v_coord.x*2.0 - 1.0;
    mediump float y = v_coord.y*2.0 - 1.0;
    gl_FragColor = textureCube(u_sampler0, vec3(+1, -y, -x));
}
