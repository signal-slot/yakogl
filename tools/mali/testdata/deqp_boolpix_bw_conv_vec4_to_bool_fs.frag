precision mediump float;
precision mediump int;

bool isOk (bool a, bool b)   { return (a == b); }
varying vec4 in0;
uniform bool ref_out0;
bool out0;


void main()
{
    
    out0 = bool(in0);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
