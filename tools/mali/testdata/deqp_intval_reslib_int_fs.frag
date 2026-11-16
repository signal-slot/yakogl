precision mediump float;
precision mediump int;

bool isOk (int a, int b)     { return (a == b); }
varying float in0;
uniform int ref_out0;
int out0;


void main()
{
    
    out0 = int(in0);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
