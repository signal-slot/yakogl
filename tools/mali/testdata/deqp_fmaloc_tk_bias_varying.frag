varying mediump vec2 v_texCoord;
varying mediump float v_lodBias;
uniform lowp sampler2D u_sampler;

void main()
{
	gl_FragColor = texture2D(u_sampler, v_texCoord, v_lodBias);
}
