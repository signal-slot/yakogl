precision mediump float;
bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
varying vec4 in0;
uniform float ref_out0;
float out0;


float sum(vec4 v) { return (v.x + v.y + v.z + v.w); }

float func (float a, vec3 b, vec2 c, vec2 d, vec4 e)
{
    return -sum(vec4(a, b) + vec4(c, d)) + sum(e);
}

void main()
{
    
    out0 = func(in0.y, in0.xzw, in0.wz, in0.yx, in0);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
