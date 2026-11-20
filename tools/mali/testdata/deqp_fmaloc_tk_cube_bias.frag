varying mediump vec3 v_texCoord;
varying mediump float v_lodBias;
uniform lowp samplerCube u_sampler;

void main()
{
	gl_FragColor = textureCube(u_sampler, v_texCoord, v_lodBias);
}
