precision mediump int;
uniform int ui_four;
varying mediump float var[4];

void main()
{
    mediump float res = float(0.0);
    for (int i = 0; i < ui_four; i++)
        res += var[i];
    gl_FragColor = vec4(res, 0.0, 0.0, 1.0);
}
