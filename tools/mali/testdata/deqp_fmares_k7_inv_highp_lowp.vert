attribute highp vec4 a_input;
varying lowp vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    lowp vec4 unrelated0 = a_input + vec4(0.1, 0.2, 0.3, 0.4);
    lowp vec4 unrelated1 = vec4(0.8, 0.4, -0.2, 0.3) * unrelated0.xywz + unrelated0;
    lowp vec4 unrelated2 = refract(unrelated1, unrelated0, distance(unrelated0, unrelated1));
    v_unrelated = a_input + 0.02 * vec4(fract(unrelated2).xyz, 0.0);
    highp vec4 used0 = a_input + vec4(0.1, 0.2, 0.3, 0.4);
    highp vec4 used1 = vec4(0.8, 0.4, -0.2, 0.3) * used0.xywz + used0;
    highp vec4 used2 = refract(used1, used0, distance(used0, used1));
    gl_Position = a_input + 0.02 * vec4(fract(used2).xyz, 0.0);
}
