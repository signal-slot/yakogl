precision mediump float;
attribute highp vec4 dEQP_Position;
attribute vec4 in0;
varying float out0;


float sum(vec4 v) { return (v.x + v.y + v.z + v.w); }

float func (float a, vec3 b, vec2 c, vec2 d, vec4 e)
{
    return -sum(vec4(a, b) + vec4(c, d)) + sum(e);
}

void main()
{
    
    out0 = func(in0.y, in0.xzw, in0.wz, in0.yx, in0);
    gl_Position = dEQP_Position;

}
