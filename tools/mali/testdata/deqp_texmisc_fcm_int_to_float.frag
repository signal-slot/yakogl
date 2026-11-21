precision mediump float;
precision mediump int;

bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
varying float v_in0;
uniform float ref_out0;
float out0;


void main()
{
    int in0 = int(v_in0 * 1.0025);

    out0 = float(in0);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
