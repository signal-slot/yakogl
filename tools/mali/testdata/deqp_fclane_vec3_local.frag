varying mediump vec4 v_coords;

void main()
{
    mediump vec4 coords = v_coords;
    mediump mat3 tmp;
    for (int i = 0; i < 3; i++)
    {
        tmp[i] = vec3(coords);
        coords = coords.yzwx * 0.5;
    }
    mediump vec3 res = vec3(0.0);
    for (int i = 0; i < 3; i++)
        res += tmp[i];
    gl_FragColor = vec4(res, 1.0);
}
