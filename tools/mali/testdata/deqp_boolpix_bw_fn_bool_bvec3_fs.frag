precision mediump float;
bool isOk (bool a, bool b)   { return (a == b); }
varying vec3 v_in0;
uniform bool ref_out0;
bool out0;


bool func (bvec3 a)
{
    return (a.x == a.y) == a.z;
}

void main()
{
    bvec3 in0 = bvec3(v_in0);

    out0 = func(in0);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
