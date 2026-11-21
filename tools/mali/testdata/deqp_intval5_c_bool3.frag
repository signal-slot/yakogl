varying mediump float v_vtxOut;

uniform bool u_a;
uniform bool u_b;
uniform bool u_c;

void main (void)
{
    mediump float result = v_vtxOut;
    bool RES = u_a && u_b && u_c;
    gl_FragColor = vec4(float(RES) * result, 0.0, 0.0, 1.0);
}
