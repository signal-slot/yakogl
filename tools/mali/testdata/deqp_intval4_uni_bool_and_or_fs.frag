varying mediump float v_vtxOut;

uniform bool u_a;
uniform bool u_b;

void main (void)
{
    mediump float result = v_vtxOut;
    gl_FragColor = vec4(float(u_a && u_b), float(u_a || u_b), float(u_a != u_b) * result, 1.0);
}
