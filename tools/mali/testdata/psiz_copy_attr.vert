/* gl_PointSize from an attribute, with a varying: its two position programs, against a triangle draw
 * (psiz_copy_attr.expect.json) and a point draw (psiz_copy_attr_points.expect.json) of the shim. */
attribute highp vec4 p; attribute highp vec4 d; varying mediump vec4 v;
void main(){ gl_Position = p; v = d; gl_PointSize = d.y + 1.0; }
