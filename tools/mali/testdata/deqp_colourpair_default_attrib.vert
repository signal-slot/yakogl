attribute highp vec4 a_position;
attribute highp float a_value;
varying highp vec4 v_color;
void main (void)
{
    gl_Position = a_position;

    highp float normalizedValue = (a_value) / float(10);
    highp float positiveNormalizedValue = (normalizedValue + float(1.0)) / 2.0;
    v_color = vec4(positiveNormalizedValue, 0.0, 0.0, 1.0);
}
