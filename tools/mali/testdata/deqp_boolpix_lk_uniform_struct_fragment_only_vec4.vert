attribute highp vec4 dEQP_Position;

struct Struct {mediump vec4 a; mediump vec4 b;};
uniform Struct val;
void main()
{
    gl_Position = dEQP_Position;

}
