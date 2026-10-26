#version 100
precision mediump float;

varying vec2 v_coords;
uniform sampler2D u_tex;
uniform vec2 u_half_pixel;
uniform float u_offset;

// # Mathematical Formulation
// 5-tap weighted downsample: center tap (weight 4.0) + 4 diagonal taps (weight 1.0 each)
// Preserves luminance energy and prevents aliasing or blockiness during downsampling.
// Total weight = 4.0 + 4 * 1.0 = 8.0, normalized by 0.125.

void main() {
    vec2 offset = u_half_pixel * u_offset;
    vec4 sum = texture2D(u_tex, v_coords) * 4.0;
    sum += texture2D(u_tex, v_coords + vec2(-offset.x, -offset.y));
    sum += texture2D(u_tex, v_coords + vec2( offset.x, -offset.y));
    sum += texture2D(u_tex, v_coords + vec2(-offset.x,  offset.y));
    sum += texture2D(u_tex, v_coords + vec2( offset.x,  offset.y));
    gl_FragColor = sum * 0.125;
}
