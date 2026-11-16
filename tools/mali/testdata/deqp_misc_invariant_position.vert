/* deqp_misc_invariant_position: an invariant gl_Position copied from an attribute, nothing else (mali-deqp-misc.md section 1: the viewport transform of an invariant position). */
attribute vec4 a_pos;
invariant gl_Position;
void main() { gl_Position = a_pos; }
