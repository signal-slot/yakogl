varying mediump float v_vtxOut;

uniform bool u_a;
uniform bool u_b;

void main (void)
{
    mediump float result = v_vtxOut;
    bool RES = !u_a && u_b;
    gl_FragColor = vec4(float(RES) * result, float(!u_b) * result, 0.0, 1.0);
}
