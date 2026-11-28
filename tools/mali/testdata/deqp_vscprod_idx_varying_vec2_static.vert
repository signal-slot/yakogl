attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec2 var[4];

void main()
{
    gl_Position = a_position;
    mediump vec2 coords = vec2(a_coords);
    for (int i = 0; i < 4; i++)
    {
        var[i] = vec2(coords);
        coords = coords * 0.5;
    }
}
