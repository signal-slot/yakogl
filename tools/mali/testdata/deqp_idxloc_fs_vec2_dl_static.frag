varying mediump vec4 v_coords;
uniform mediump int ui_two;

void main()
{
    mediump vec2 coords = vec2(v_coords);
    mediump vec2 tmp;
    for (int i = 0; i < ui_two; i++)
    {
        tmp[i] = coords.x;
        coords = coords.yx * 0.5;
    }
    mediump float res = 0.0;
    res += tmp[0];
    res += tmp[1];
    gl_FragColor = vec4(vec3(res), 1.0);
}
