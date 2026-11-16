varying mediump vec4 v_coords;
uniform mediump float uf_one;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    for (mediump float ndx = 0.05; ndx < 1.0; ndx += 0.333333)
    {
        res = res.yzwx;
    }
    gl_FragColor = vec4(res.rgb, 1.0);
}
