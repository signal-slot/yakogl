attribute highp vec4 dEQP_Position;
attribute mat3 in0;

varying mediump mat3 var;
void main()
{
    var = in0;
    gl_Position = dEQP_Position;

}
