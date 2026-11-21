varying mediump float v_vtxOut;

uniform mediump ivec4 u_a;

void main (void)
{
    mediump float result = v_vtxOut;
    bool RES = (u_a == ivec4(-1, 8, -4, 10));
    gl_FragColor = vec4(float(RES) * result, 0.0, 0.0, 1.0);
}
