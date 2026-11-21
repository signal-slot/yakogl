attribute highp vec4 a_input;
varying mediump vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    v_unrelated = a_input.xzxz + (1.0e20*a_input.x*a_input.xxxx + 1.0e20*a_input.y*a_input.yyyy) * (1.08 * a_input.zyzy * a_input.xzxz) * 1.0e-20 * (a_input.z * a_input.zzxz - a_input.z * a_input.zzxz) + (1.0e20*a_input.x*a_input.xxxx + 1.0e20*a_input.y*a_input.yyyy) / 1.0e20;
    gl_Position = a_input + (1.0e20*a_input.x*a_input.xxxx + 1.0e20*a_input.y*a_input.yyyy) * 1.0e-20;
}
