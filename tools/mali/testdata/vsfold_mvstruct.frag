precision highp float;
varying vec4 vPosition;
varying vec4 vPrevPosition;
void main()
{
    gl_FragColor = vPosition - vPrevPosition;
}
