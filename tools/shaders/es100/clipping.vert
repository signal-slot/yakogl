/* dEQP-GLES2.functional.clipping: shaderSourceVertex of VK-GL-CTS
 * modules/gles2/functional/es2fClippingTests.cpp (Apache-2.0). */
attribute highp vec4 a_position;
attribute highp vec4 a_color;
attribute highp float a_pointSize;
varying mediump vec4 varFragColor;
void main (void)
{
    gl_Position = a_position;
    gl_PointSize = a_pointSize;
    varFragColor = a_color;
}
