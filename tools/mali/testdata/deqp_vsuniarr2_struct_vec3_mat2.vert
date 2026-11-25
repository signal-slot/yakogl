attribute highp vec4 a_position;
varying mediump float v_vtxOut;

struct structType1
{
	mediump vec3 m0;
};
struct structType2
{
	mediump vec3 m0[1];
};
struct structType0
{
	structType1 m0;
	structType2 m1;
	mediump mat3 m2;
	mediump mat2 m3[3];
};
uniform mediump mat3 u_var0;
uniform structType0 u_var1;

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat3     (mediump mat3 a, mediump mat3 b)    { return compare_vec3(a[0], b[0])*compare_vec3(a[1], b[1])*compare_vec3(a[2], b[2]); }

void main (void)
{
    gl_Position = a_position;
    v_vtxOut = 1.0;
	v_vtxOut *= compare_mat3(u_var0, mat3(-3.20, -3.68, 8.57, 6.88, 8.99, -9.95, 9.51, -5.64, -5.94));
	// UNUSED: u_var1.m0.m0
	v_vtxOut *= compare_vec3(u_var1.m1.m0[0], vec3(2.99, -4.67, -2.46));
	// UNUSED: u_var1.m2
	v_vtxOut *= compare_mat2(u_var1.m3[0], mat2(-8.07, -2.94, -1.62, -9.48));
	// UNUSED: u_var1.m3[1]
	v_vtxOut *= compare_mat2(u_var1.m3[2], mat2(6.86, 5.56, -9.34, -2.42));
}
