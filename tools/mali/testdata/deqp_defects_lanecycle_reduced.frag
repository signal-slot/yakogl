/* dEQP-GLES2.functional.shaders.random.scalar_conversion.fragment.96 cut to the colour that sends
 * nir_fs::lane_tree and nir_fs::vectorise into each other without end
 * at f9b37651: lane w, (2.0 * b) * 0.0, is the constant 0.0 and lane z,
 * (2.0 * b) * b, is computed. Defect `lanecycle` of
 * docs/notes/mali-deqp-defects.md. */
precision mediump float;
varying mediump float b;

void main()
{
	gl_FragColor = vec4(2.0) * b * vec4(0.0, 0.0, b, 0.0);
}
