uniform mediump vec4 vTest;
uniform mediump mat4 vMatrix;
void main (void)
{
    gl_Position = vMatrix * vTest;
}
