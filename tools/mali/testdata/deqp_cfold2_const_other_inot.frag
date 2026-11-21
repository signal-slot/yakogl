precision mediump int;
varying mediump vec4 var[4];

void main()
{
    mediump vec4 res = vec4(0.0);
    res += var[0];
    res += var[1];
    res += var[2];
    res += var[3];
    gl_FragColor = vec4(res);
}
