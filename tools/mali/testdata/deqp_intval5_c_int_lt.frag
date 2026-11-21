varying mediump float v_vtxOut;

uniform mediump int u_a;
uniform mediump int u_b;

void main (void)
{
    mediump float result = v_vtxOut;
    gl_FragColor = vec4(float(u_a < u_b) * result, float(u_a >= 3) * result, 0.0, 1.0);
}
