attribute highp vec4 a_position;
attribute mediump vec4 a_unitCoords;
varying mediump vec3 v_color;

void main()
{
    gl_Position = a_position;
    mediump vec4 coords = a_unitCoords;
    mediump vec3 res = vec3(0.0);
	mediump float H = coords.x, S = coords.y, L = coords.z;
	mediump float v = (L <= 0.5) ? (L * (1.0 + S)) : (L + S - L * S);
	res = vec3(L); // default to gray
	if (v > 0.0)
	{
	    mediump float m = L + L - v;
	    mediump float sv = (v - m) / v;
	    H *= 6.0;
	    mediump int sextant = int(H);
	    mediump float fract = H - float(sextant);
	    mediump float vsf = v * sv * fract;
	    mediump float mid1 = m + vsf;
	    mediump float mid2 = m - vsf;
	    if (sextant == 0)      res = vec3(v, mid1, m);
	    else if (sextant == 1) res = vec3(mid2, v, m);
	    else if (sextant == 2) res = vec3(m, v, mid1);
	    else if (sextant == 3) res = vec3(m, mid2, v);
	    else if (sextant == 4) res = vec3(mid1, m, v);
	    else                   res = vec3(v, m, mid2);
	}
    v_color = res;
}
