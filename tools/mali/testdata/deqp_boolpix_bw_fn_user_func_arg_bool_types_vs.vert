precision mediump float;
attribute highp vec4 dEQP_Position;
attribute float a_in0;
varying float v_out0;
bool out0;


bool func (bool a) { return !a; }
bvec2 func (bvec2 a) { return a.yx; }
bvec3 func (bvec3 a) { return a.xxx; }
bvec4 func (bvec4 a) { return a.wwww; }

void main()
{
    bool in0 = bool(a_in0);

    out0 = func(func(func(func(bvec4(in0)).xyz).xy).x);
    gl_Position = dEQP_Position;
v_out0 = float(out0);

}
