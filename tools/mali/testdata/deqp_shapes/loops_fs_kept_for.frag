varying mediump vec4 v_coords;
varying mediump float v_one;
uniform highp float uf_third;
uniform highp float uf_one;

void main()
{
    mediump vec4 coords = v_coords;
    highp float one = v_one;
    mediump vec4 res = coords;
    for (highp float ndx = 0.05; ndx < uf_one*one; ndx += uf_third*one)
    {
        res = res.yzwx;
    }
    gl_FragColor = vec4(res.rgb, 1.0);
}
