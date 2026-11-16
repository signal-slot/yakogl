/* gl_PointSize from a constant, no varying program: its two position programs, against a triangle draw
 * (psiz_copy_alone.expect.json) and a point draw (psiz_copy_alone_points.expect.json) of the shim. */
attribute highp vec4 p;
void main(){ gl_Position = p; gl_PointSize = 2.0; }
