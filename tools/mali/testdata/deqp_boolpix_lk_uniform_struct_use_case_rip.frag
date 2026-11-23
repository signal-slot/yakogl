precision mediump float;
struct Light
{
    mediump vec3    color;
    highp vec4        position;
    highp vec3        direction;
    mediump float    constantAttenuation;
    mediump float    linearAttenuation;
    mediump float    quadraticAttenuation;
};
struct Struct {float a;};
uniform Light val;
bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
uniform float ref_out0;
float out0;

varying mediump float res;
void main()
{
   out0 = res + val.quadraticAttenuation;
   bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
