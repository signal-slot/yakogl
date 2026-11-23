attribute highp vec4 a_position;
varying mediump float v_vtxOut;

struct subSubStructType
{
	mediump sampler2D mss0;
	mediump samplerCube mss1;
};
struct subStructType
{
	mediump samplerCube ms0;
	mediump sampler2D ms1[2];
	subSubStructType ms2[2];
};
struct structType
{
	mediump sampler2D m0;
	subStructType m1;
	mediump samplerCube m2;
};
uniform structType u_var;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_vec4(texture2D(u_var.m0, vec2(0.0)), vec4(0.15, 0.52, 0.26, 0.35));
	v_vtxOut *= compare_vec4(textureCube(u_var.m1.ms0, vec3(0.0)), vec4(0.88, 0.09, 0.30, 0.61));
	v_vtxOut *= compare_vec4(texture2D(u_var.m1.ms1[0], vec2(0.0)), vec4(0.85, 0.59, 0.33, 0.71));
	v_vtxOut *= compare_vec4(texture2D(u_var.m1.ms1[1], vec2(0.0)), vec4(0.62, 0.89, 0.09, 0.99));
	v_vtxOut *= compare_vec4(texture2D(u_var.m1.ms2[0].mss0, vec2(0.0)), vec4(0.53, 0.89, 0.01, 0.08));
	v_vtxOut *= compare_vec4(textureCube(u_var.m1.ms2[0].mss1, vec3(0.0)), vec4(0.26, 0.72, 0.60, 0.12));
	v_vtxOut *= compare_vec4(texture2D(u_var.m1.ms2[1].mss0, vec2(0.0)), vec4(0.05, 0.45, 0.94, 0.80));
	v_vtxOut *= compare_vec4(textureCube(u_var.m1.ms2[1].mss1, vec3(0.0)), vec4(0.58, 0.21, 0.32, 0.34));
	v_vtxOut *= compare_vec4(textureCube(u_var.m2, vec3(0.0)), vec4(0.89, 0.99, 0.06, 0.17));
}
