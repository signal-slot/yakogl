attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 var[4];

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = vec4(a_coords);
    for (int i = 0; i < 4; i++)
    {
        var[i] = vec4(coords);
        coords = coords * 0.5;
    }
}
