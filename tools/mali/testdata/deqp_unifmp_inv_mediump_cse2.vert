attribute mediump vec4 a_input;
varying mediump vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    mediump vec4 a = 1.0e2 * (a_input.xxxx + a_input.yyyy);
    mediump vec4 b = (1.0e2 * (a_input.xxxx + a_input.yyyy)) * (1.0e2 * (a_input.xxxx + a_input.yyyy)) / 1.0e2 / 1.0e2;
    mediump vec4 c = a * a;
    mediump vec4 d = c / 1.0e2 / 1.0e2;
    v_unrelated = a + b + c + d;
    gl_Position = a_input + d;
}
