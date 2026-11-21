attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec3 v_color;

void main()
{
    gl_Position = a_position;
    mediump vec2 coords = vec2(a_coords);
    mediump vec2 tmp;
    for (int i = 0; i < 2; i++)
    {
        tmp[i] = coords.x;
        coords = coords.yx * 0.5;
    }
    mediump float res = 0.0;
    res = dot(tmp, vec2(1.0));
    v_color = vec3(res);
}
