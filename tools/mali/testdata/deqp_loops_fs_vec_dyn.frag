varying mediump vec4 v_coords;
uniform mediump int ui_four;

void main()
{
    mediump vec4 coords = vec4(v_coords);
    mediump vec4 tmp;
    for (int i = 0; i < ui_four; i++)
    {
        tmp[i] = coords.x;
        coords = coords.yzwx * 0.5;
    }
    mediump float res = 0.0;
    for (int i = 0; i < ui_four; i++)
        res += tmp[i];
    gl_FragColor = vec4(vec3(res), 1.0);
}
