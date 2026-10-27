/* The clear colour, from the one uniform the backend writes per clear (four
 * floats at the start of the block). The ES 1.00 twin of ../clear.frag; the
 * PowerVR's masked clears are its blend variants with the channels left off
 * (glColorMask is lowered into the fragment shader there). */
precision mediump float;
uniform vec4 u_color;
void main() {
    gl_FragColor = u_color;
}
