/* The back ends' own full-screen quad (the PowerVR's frame load and clear
 * quads, the GC7000's clear quad): the backend writes the corners in clip
 * space, z already where the draw wants it. The ES 1.00 twin of
 * ../fbload.vert and ../clear.vert, which Mesa compiles. */
attribute vec4 a_pos;
void main() {
    gl_Position = a_pos;
}
