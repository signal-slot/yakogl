struct Light
{
    mediump vec3    color;
    highp vec4        position;
    highp vec3        direction;
    mediump float    constantAttenuation;
    mediump float    linearAttenuation;
    mediump float    quadraticAttenuation;
};
attribute highp vec4 dEQP_Position;

uniform Light val;
varying mediump float res;
void main()
{
   res = val.constantAttenuation;
   gl_Position = dEQP_Position;

}
