varying mediump vec4 v_coords;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec3 res = vec3(0.0);
	mediump float r = coords.x, g = coords.y, b = coords.z;
	mediump float minVal = min(min(r, g), b);
	mediump float maxVal = max(max(r, g), b);
	mediump float L = (minVal + maxVal) * 0.5;
	if (minVal == maxVal)
	    res = vec3(0.0, 0.0, L);
	else
	{
	    mediump float H;
	    mediump float S;
	    if (L < 0.5)
	        S = (maxVal - minVal) / (maxVal + minVal);
	    else
	        S = (maxVal - minVal) / (2.0 - maxVal - minVal);
	
	    mediump float ooDiff = 1.0 / (maxVal - minVal);
	    if (r == maxVal)      H = (g - b) * ooDiff;
	    else if (g == maxVal) H = 2.0 + (b - r) * ooDiff;
	    else                  H = 4.0 + (r - g) * ooDiff;
	    H /= 6.0;
	
	    res = vec3(H, S, L);
	}
    gl_FragColor = vec4(res, 1.0);
}
