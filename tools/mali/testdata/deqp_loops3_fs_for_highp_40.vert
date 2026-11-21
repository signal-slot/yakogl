attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying highp vec4 v;

void main()
{
    gl_Position = a_position;
    v = a_coords;
}
