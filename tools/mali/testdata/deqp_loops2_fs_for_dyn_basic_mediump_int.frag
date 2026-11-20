varying mediump vec4 v_coords;
varying mediump float v_one;
uniform mediump int ui_three;

void main()
{
    mediump vec4 coords = v_coords;
    mediump int one = int(v_one + 0.5);
    mediump vec4 res = coords;
    for (mediump int ndx = 0; ndx < ui_three*one; ndx++)
    {
        res = res.yzwx;
    }
    gl_FragColor = vec4(res.rgb, 1.0);
}
