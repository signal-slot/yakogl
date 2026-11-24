#version 300 es
in mediump vec3 v_texCoord;
in mediump float v_lodBias;
uniform lowp samplerCube u_sampler;
layout(location = 0) out mediump vec4 o_color;

void main()
{
	o_color = textureLod(u_sampler, v_texCoord, v_lodBias);
}
