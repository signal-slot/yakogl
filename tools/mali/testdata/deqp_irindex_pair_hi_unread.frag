precision mediump float;

bool isOk (bool a, bool b)   { return (a == b); }
varying vec2 v_in0;
uniform bool ref_out0;
bool out0;


void main()
{
    bvec2 in0 = bvec2(v_in0);

    out0 = in0.x;
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
