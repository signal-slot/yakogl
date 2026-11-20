attribute highp vec4 a_coords;
uniform mediump int ui_one;
attribute highp vec4 a_position;
varying mediump vec4 v_color;

highp vec4 getCoords (void)
{
    highp vec4 coords = a_coords;
    for (int i = 0; i < ui_one; i++)
    {
        if (false)
            return coords;
        coords = coords.wzyx;
    }
    return coords;
}

void main (void)
{
    gl_Position = a_position;
    v_color = vec4(getCoords().xyz, 1.0);
}
