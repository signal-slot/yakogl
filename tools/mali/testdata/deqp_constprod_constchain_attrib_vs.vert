attribute highp vec4 a_position;
attribute highp vec4 a_value;
varying highp vec4 v_color;
void main (void)
{
    gl_Position = a_position;

    highp vec4 normalizedValue = (a_value) / float(10);
    highp vec4 positiveNormalizedValue = (normalizedValue + vec4(1.0)) / 2.0;
    v_color = vec4((positiveNormalizedValue.xy + positiveNormalizedValue.zz) / 2.0, positiveNormalizedValue.w, 1.0);
}
