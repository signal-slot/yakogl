varying mediump vec4 v_coords;
varying mediump float v_one;
uniform mediump float uf_third;
uniform mediump float uf_one;

void main()
{
    mediump vec4 coords = v_coords;
    mediump float one = v_one;
    mediump vec4 res = coords;
    for (mediump float ndx = 0.05; ndx < uf_one*one; ndx += uf_third*one)
    {
        res = res.yzwx;
    }
    gl_FragColor = vec4(res.rgb, 1.0);
}
