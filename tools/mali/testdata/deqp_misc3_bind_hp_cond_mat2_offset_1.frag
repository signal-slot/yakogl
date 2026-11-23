#version 100
/* dEQP-GLES2.functional.attribute_location.bind_aliasing.cond_mat2_offset_1: the fragment shader, as dEQP's
 * generateVertexShaderTemplate / BindAliasingAttributeTest write it (glsAttributeLocationTests.cpp,
 * VK-GL-CTS 93bca018, Apache-2.0), with the vertex shader's mediump made highp: its arithmetic is
 * fp32, so the words check the attribute layout at the link's locations alone (#11388); the mediump
 * vertex arithmetic of the case itself is #4536's. Drawn with the bindings of the .bind file: a_0 at 1, a_1 at 2. */

varying mediump vec4 v_color;
void main (void)
{
	gl_FragColor = v_color;
}
