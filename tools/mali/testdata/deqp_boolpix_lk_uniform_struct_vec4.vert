attribute highp vec4 dEQP_Position;

struct Struct {mediump vec4 a; mediump vec4 b;};
uniform Struct val;
varying mediump float dummy;
void main()
{
    dummy = val.a.x + val.b.y;
    gl_Position = dEQP_Position;

}
