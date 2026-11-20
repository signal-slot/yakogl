attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying highp vec4 v_color;
uniform int ui_zero;

highp vec4 getCoords (void)
{
    for (int i = 1; i < 10; i += ui_zero)
        return a_coords;
    return a_coords.wzyx;
}

void main (void)
{
    gl_Position = a_position;
    v_color = vec4(getCoords().xyz, 1.0);
    return;
}
