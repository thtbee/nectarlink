// SPDX-License-Identifier: GPL-3.0-or-later
// A phone app's picture with light and dark swapped, for apps with no dark
// mode (a white map). Brightness is turned over and colors keep their hue,
// so a blue road stays blue; white becomes a soft dark gray rather than
// black, and black a soft light gray.
//
// Compiled to darken.frag.qsb with Qt's qsb (see shaders.qrc).
#version 440

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4 qt_Matrix;
    float qt_Opacity;
};

layout(binding = 1) uniform sampler2D source;

void main() {
    vec4 c = texture(source, qt_TexCoord0);
    vec3 rgb = c.a > 0.0 ? c.rgb / c.a : vec3(0.0);
    float y = dot(rgb, vec3(0.299, 0.587, 0.114));
    float turned = 0.9 - 0.8 * y;
    vec3 dark = clamp(rgb + (turned - y), 0.0, 1.0);
    fragColor = vec4(dark * c.a, c.a) * qt_Opacity;
}
