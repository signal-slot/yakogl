attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump float var[4];

void main()
{
    gl_Position = a_position;
    mediump float coords = float(a_coords);
    for (int i = 0; i < 4; i++)
    {
        var[i] = float(coords);
        coords = coords * 0.5;
    }
}
