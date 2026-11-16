/* dEQP-GLES2.functional.shaders.operator.common_functions.abs.mediump_vec2_vertex,
 * rebuilt from VK-GL-CTS 93bca018 (Apache-2.0): modules/gles2/functional/es2fShaderOperatorTests.cpp:590-745
 * (ShaderOperatorCase, the vertex case) with the shaderOp of :1987-2081 and
 * abs's BuiltinFuncInfo at :1778 (scale 0.5, bias 0.5; in0's swizzle "wy",
 * :501). Shape P of docs/notes/mali-deqp-varylink.md. */
attribute highp vec4 a_position;
attribute mediump vec4 a_in0;
varying mediump vec4 v_color;

void main()
{
    gl_Position = a_position;
	mediump vec2 in0 = a_in0.wy;
	mediump vec2 res = vec2(0.0);

	res = abs(in0);

	mediump vec4 color = vec4(0.0, 0.0, 0.0, 1.0);
	color.yz = res;
	color = color * 0.50 + 0.50;
    v_color = color;
}
