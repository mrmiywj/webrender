#line 1

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

void main(void) {
#ifdef WR_FEATURE_TRANSFORM
    float alpha = 0.0;
    vec2 pos = init_transform_fs(vLocalPos, vLocalRect, alpha);

    // We clamp the texture coordinate calculation here to the local rectangle boundaries,
    // which makes the edge of the texture stretch instead of repeat.
    vec2 relative_pos_in_rect =
         clamp(pos, vLocalRect.xy, vLocalRect.xy + vLocalRect.zw) - vLocalRect.xy;
#else
    float alpha = 1.0;
    vec2 relative_pos_in_rect = vLocalPos;
#endif

    alpha = min(alpha, do_clip());

    // We calculate the particular tile this fragment belongs to, taking into
    // account the spacing in between tiles. We only paint if our fragment does
    // not fall into that spacing.
    vec2 position_in_tile = mod(relative_pos_in_rect, vStretchSize + vTileSpacing);
    // We clamp the texture coordinates to the half-pixel offset from the borders
    // in order to avoid sampling outside of the texture area.
    vec2 st = vTextureOffset + ((position_in_tile / vStretchSize) * vTextureSize);
    st = clamp(st, vStRect.xy, vStRect.zw);

    alpha = alpha * float(all(bvec2(step(position_in_tile, vStretchSize))));

    oFragColor = vec4(alpha) * textureLod(sColor0, st, 0.0);
}
