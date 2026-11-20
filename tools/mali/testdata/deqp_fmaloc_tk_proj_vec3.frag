varying mediump vec3 v_texCoord;
uniform lowp sampler2D u_sampler;

void main()
{
	gl_FragColor = texture2DProj(u_sampler, v_texCoord);
}
