varying highp vec3 v_texCoord;
uniform highp float u_bias;
uniform highp float u_ref;
uniform highp vec4 u_colorScale;
uniform highp vec4 u_colorBias;
uniform highp samplerCube u_sampler;

void main (void)
{
    gl_FragColor = textureCube(u_sampler, v_texCoord, u_bias) * u_colorScale + u_colorBias;
}
