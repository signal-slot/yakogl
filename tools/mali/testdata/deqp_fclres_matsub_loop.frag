varying mediump vec4 v_coords;

void main()
{
    mediump vec4 coords = v_coords;
    mediump mat4 tmp;
    for (int i = 0; i < 4; i++)
    {
        tmp[i] = vec4(coords);
        coords = coords.yzwx * 0.5;
    }
    mediump vec4 res = vec4(0.0);
    res += tmp[0];
    res += tmp[1];
    res += tmp[2];
    res += tmp[3];
    gl_FragColor = vec4(res);
}
