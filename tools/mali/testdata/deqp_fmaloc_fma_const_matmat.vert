precision highp float;
attribute highp vec4 dEQP_Position;
varying float out0;


void main()
{
    const mat3 a = mat3(3.0) * mat3(4.0);
    const mat4 b = mat4(a[1][1])*2.0;
    const mat2 c = mat2(b[0][0]) / 3.0;
    out0 = c[0][0]+c[1][0];
    gl_Position = dEQP_Position;

}
