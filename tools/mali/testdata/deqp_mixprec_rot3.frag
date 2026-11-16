varying mediump vec4 v_coords;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    for (mediump int ndx = 0; ndx < 3; ndx++)
    {
        res = res.yzwx;
    }
    gl_FragColor = vec4(res.rgb, 1.0);
}
