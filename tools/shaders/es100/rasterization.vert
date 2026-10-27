/* dEQP-GLES2.functional.rasterization: s_shaderVertexTemplate of VK-GL-CTS
 * modules/gles2/functional/es2fRasterizationTests.cpp (Apache-2.0). Its
 * varying is highp here and mediump in the fragment shader. */
attribute highp vec4 a_position;
attribute highp vec4 a_color;
varying highp vec4 v_color;
uniform highp float u_pointSize;
void main ()
{
    gl_Position = a_position;
    gl_PointSize = u_pointSize;
    v_color = a_color;
}
