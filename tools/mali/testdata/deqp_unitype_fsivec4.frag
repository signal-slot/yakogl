uniform mediump ivec4 fTest;
uniform sampler2D fSampler;
void main (void)
{
    gl_FragColor.xy = vec4(fTest).xy;
    gl_FragColor.zw = texture2D(fSampler, vec2(0.0, 0.0)).zw;
}
