/* LOCALS (original reduction, glmark2 effect2d/terrain shape): 36 vec4 locals,
 * each one a product of two attribute sums with a uniform (no common factor
 * for Mesa to pull out of the sum), summed into the position.
 * The front end gives each declaration one local (lower.rs alloc_var), so the
 * 33rd one is the TooManyLocals refusal (nir.rs build, MAX_LOCALS = 32). */
attribute vec4 p;
attribute vec4 q;
uniform vec4 k[36];
void main()
{
	vec4 a0 = (p + k[0]) * (q + k[0]);
	vec4 a1 = (p + k[1]) * (q + k[1]);
	vec4 a2 = (p + k[2]) * (q + k[2]);
	vec4 a3 = (p + k[3]) * (q + k[3]);
	vec4 a4 = (p + k[4]) * (q + k[4]);
	vec4 a5 = (p + k[5]) * (q + k[5]);
	vec4 a6 = (p + k[6]) * (q + k[6]);
	vec4 a7 = (p + k[7]) * (q + k[7]);
	vec4 a8 = (p + k[8]) * (q + k[8]);
	vec4 a9 = (p + k[9]) * (q + k[9]);
	vec4 a10 = (p + k[10]) * (q + k[10]);
	vec4 a11 = (p + k[11]) * (q + k[11]);
	vec4 a12 = (p + k[12]) * (q + k[12]);
	vec4 a13 = (p + k[13]) * (q + k[13]);
	vec4 a14 = (p + k[14]) * (q + k[14]);
	vec4 a15 = (p + k[15]) * (q + k[15]);
	vec4 a16 = (p + k[16]) * (q + k[16]);
	vec4 a17 = (p + k[17]) * (q + k[17]);
	vec4 a18 = (p + k[18]) * (q + k[18]);
	vec4 a19 = (p + k[19]) * (q + k[19]);
	vec4 a20 = (p + k[20]) * (q + k[20]);
	vec4 a21 = (p + k[21]) * (q + k[21]);
	vec4 a22 = (p + k[22]) * (q + k[22]);
	vec4 a23 = (p + k[23]) * (q + k[23]);
	vec4 a24 = (p + k[24]) * (q + k[24]);
	vec4 a25 = (p + k[25]) * (q + k[25]);
	vec4 a26 = (p + k[26]) * (q + k[26]);
	vec4 a27 = (p + k[27]) * (q + k[27]);
	vec4 a28 = (p + k[28]) * (q + k[28]);
	vec4 a29 = (p + k[29]) * (q + k[29]);
	vec4 a30 = (p + k[30]) * (q + k[30]);
	vec4 a31 = (p + k[31]) * (q + k[31]);
	vec4 a32 = (p + k[32]) * (q + k[32]);
	vec4 a33 = (p + k[33]) * (q + k[33]);
	vec4 a34 = (p + k[34]) * (q + k[34]);
	vec4 a35 = (p + k[35]) * (q + k[35]);
	vec4 s = a0;
	s += a1;
	s += a2;
	s += a3;
	s += a4;
	s += a5;
	s += a6;
	s += a7;
	s += a8;
	s += a9;
	s += a10;
	s += a11;
	s += a12;
	s += a13;
	s += a14;
	s += a15;
	s += a16;
	s += a17;
	s += a18;
	s += a19;
	s += a20;
	s += a21;
	s += a22;
	s += a23;
	s += a24;
	s += a25;
	s += a26;
	s += a27;
	s += a28;
	s += a29;
	s += a30;
	s += a31;
	s += a32;
	s += a33;
	s += a34;
	s += a35;
	gl_Position = s;
}
