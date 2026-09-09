#version 150 core

// A figure's colour, either read from the ramp or the one flat colour a figure
// takes where nothing varies across it.
//
// The ramp holds the colour along one coordinate, sampled with repeating wrap,
// so the sawtooth's discontinuity lands exactly where it belongs however coarse
// the mesh is and the cycle count never reaches the geometry.
//
// The texture is stored as sRGB and the framebuffer encodes on write, so what
// arrives from the sampler is linear and what goes out is converted back. A
// flat colour is written out as it came in, which is what a figure drawn
// through piston's own triangle list does with it.

uniform sampler2D u_ramp;
uniform int u_flat;
uniform vec4 u_color;

in vec2 v_uv;

out vec4 o_Color;

void main() {
    o_Color = u_flat != 0 ? u_color : texture(u_ramp, v_uv);
}
