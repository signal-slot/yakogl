varying mediump vec4 v_coords;
uniform mediump int ui_four;

void main()
{
    mediump vec4 coords = v_coords;
    mediump mat4 tmp;
    tmp[0] = vec4(coords);
    tmp[1] = vec4(coords.yzwx) * 0.5;
    tmp[2] = vec4(coords.zwxy) * 0.25;
    tmp[3] = vec4(coords.wxyz) * 0.125;
    mediump vec4 res = vec4(0.0);
    for (int i = 0; i < ui_four; i++)
        res += tmp[i];
    gl_FragColor = vec4(res);
}
