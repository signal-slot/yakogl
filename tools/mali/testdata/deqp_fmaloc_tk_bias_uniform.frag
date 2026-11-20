varying mediump vec2 v_texCoord;
uniform mediump float u_bias;
uniform mediump float u_ref;
uniform mediump vec4 u_colorScale;
uniform mediump vec4 u_colorBias;
uniform mediump sampler2D u_sampler;

void main (void)
{
    gl_FragColor = texture2D(u_sampler, v_texCoord, u_bias) * u_colorScale + u_colorBias;
}
