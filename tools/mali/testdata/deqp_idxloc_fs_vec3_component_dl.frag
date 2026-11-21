varying mediump vec4 v_coords;
uniform mediump int ui_three;

void main()
{
    mediump vec3 coords = vec3(v_coords);
    mediump vec3 tmp;
    tmp.x = coords.x;
    tmp.y = coords.y * 0.5;
    tmp.z = coords.z * 0.25;
    mediump float res = 0.0;
    for (int i = 0; i < ui_three; i++)
        res += tmp[i];
    gl_FragColor = vec4(vec3(res), 1.0);
}
