attribute highp vec4 a_input;
varying highp vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    highp vec4 value = a_input;
    v_unrelated = vec4(0.0, 0.0, 0.0, 0.0);
    for (mediump int i = 0; i < 14; ++i)
    {
        value *= 1.9;
        v_unrelated += value;
    }
    gl_Position = vec4(value.xyz / 1.0e4 + a_input.xyz * 0.1, 1.0);
}
