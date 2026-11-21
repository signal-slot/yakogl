varying mediump float v_vtxOut;

uniform mediump int u_a;
uniform mediump int u_b;
uniform mediump int u_c;

void main (void)
{
    mediump float result = v_vtxOut;
    bool RES = (u_a == -9) && (u_b == 2) && (u_c == -2);
    gl_FragColor = vec4(float(RES) * result, 0.0, 0.0, 1.0);
}
