varying mediump float v_vtxOut;

uniform mediump int u_a;
uniform mediump int u_b;

void main (void)
{
    mediump float result = v_vtxOut;
    bool RES = (u_a == 2) && (u_b == -3);
    gl_FragColor = vec4(float(RES) * result, 0.0, 0.0, 1.0);
}
