attribute highp vec4 dEQP_Position;

struct Struct {mediump float a; mediump float b;};
uniform Struct val;
varying mediump float res;
void main()
{
    res = val.a;
    gl_Position = dEQP_Position;

}
