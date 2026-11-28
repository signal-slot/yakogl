uniform highp sampler2D uniformSampler;
void main (void)
{
    gl_FragColor = vec4(texture2D(uniformSampler, vec2(0.0, 0.0)).x);
}
