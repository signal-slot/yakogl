attribute highp vec4 dEQP_Position;

struct Struct {mediump float a; mediump float b;};
uniform Struct val;
varying mediump float dummy;
void main()
{
    dummy = val.a + val.b;
    gl_Position = dEQP_Position;

}
