precision mediump float;

bool isOk (ivec4 a, ivec4 b) { return (a == b); }
varying vec2 v_in0;
uniform ivec4 ref_out0;
ivec4 out0;


void main()
{
    ivec2 in0 = ivec2(v_in0 * 1.0025);

    out0 = in0.gggg;
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
