#version 100
precision mediump float;

varying vec2 v_coords;
uniform sampler2D u_tex;
uniform vec2 u_half_pixel;
uniform float u_offset;

// # Mathematical Formulation
// 9-tap isotropic Dual-Kawase blur kernel:
// Center tap: weight 4.0
// 4 Diagonal taps at (±dx, ±dy): weight 2.0 each
// 4 Orthogonal taps at (±dx*1.414, 0) and (0, ±dy*1.414): weight 1.0 each
// Total weight = 4.0 + 4*2.0 + 4*1.0 = 16.0
// Normalization: 1.0 / 16.0 = 0.0625

void main() {
    vec2 d = u_half_pixel * u_offset;
    vec2 d_ortho = d * 1.41421356;

    vec4 col = texture2D(u_tex, v_coords) * 4.0;

    // 4 Diagonal samples (weight 2.0 each)
    col += texture2D(u_tex, v_coords + vec2(-d.x, -d.y)) * 2.0;
    col += texture2D(u_tex, v_coords + vec2( d.x, -d.y)) * 2.0;
    col += texture2D(u_tex, v_coords + vec2(-d.x,  d.y)) * 2.0;
    col += texture2D(u_tex, v_coords + vec2( d.x,  d.y)) * 2.0;

    // 4 Orthogonal samples (weight 1.0 each)
    col += texture2D(u_tex, v_coords + vec2(-d_ortho.x, 0.0));
    col += texture2D(u_tex, v_coords + vec2( d_ortho.x, 0.0));
    col += texture2D(u_tex, v_coords + vec2(0.0, -d_ortho.y));
    col += texture2D(u_tex, v_coords + vec2(0.0,  d_ortho.y));

    gl_FragColor = col * 0.0625;
}
