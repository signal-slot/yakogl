precision mediump float;
attribute highp vec4 dEQP_Position;
attribute mat2 in0;
varying mat2 out0;


mat2 func (mat2 a)
{
    return -2.0*a;
}

void main()
{
    
    out0 = func(in0);
    gl_Position = dEQP_Position;

}
