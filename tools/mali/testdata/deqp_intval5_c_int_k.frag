varying mediump float v_vtxOut;

uniform mediump int u_a;
uniform mediump int u_b;
uniform mediump int u_c;

void main (void)
{
    mediump float result = v_vtxOut;
    gl_FragColor = vec4(float(u_a == 4) * result, float(u_b == 9) * result, float(u_c == 17) * result, 1.0);
}
