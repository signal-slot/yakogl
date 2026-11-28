varying mediump vec4 v_coords;

void main()
{
    mediump vec4 coords = vec4(v_coords);
    mediump vec4 tmp;
    tmp[0] = coords.x;
    tmp[1] = coords.y * 0.5;
    tmp[2] = coords.z * 0.25;
    tmp[3] = coords.w * 0.125;
    mediump float res = 0.0;
    for (int i = 0; i < 4; i++)
        res += tmp[i];
    gl_FragColor = vec4(vec3(res), 1.0);
}
