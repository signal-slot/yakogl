attribute highp vec4 a_position;
varying mediump float v_vtxOut;


void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
}
