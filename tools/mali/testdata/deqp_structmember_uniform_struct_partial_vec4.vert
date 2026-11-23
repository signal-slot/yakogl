attribute highp vec4 dEQP_Position;

struct Struct {mediump vec4 a; mediump vec4 b;};
uniform Struct val;
varying mediump float res;
void main()
{
    res = val.a.x;
    gl_Position = dEQP_Position;

}
