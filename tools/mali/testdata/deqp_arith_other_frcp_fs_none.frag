void main (void)
{
    gl_FragColor = vec4(0.0, 1.0/gl_FragCoord.w - 1.0, 0.0, 1.0);
}
