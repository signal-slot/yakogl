attribute highp vec4 a_position;
varying highp vec4 v_coord;
void main (void)
{
    gl_Position = a_position;
    v_coord = a_position;
}
