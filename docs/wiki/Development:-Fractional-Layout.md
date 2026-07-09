There are two main coordinate spaces in niri: physical (pixels of every individual output) and logical (shared among all outputs, takes into account the scale of every output).
Wayland clients mostly work in the logical space, and it's the most convenient space to do all the layout in, since it bakes in the output scaling factor.

However, many things need to be sized or positioned at integer physical coordinates.
For example, Wayland toplevel buffers are assumed to be placed at an integer physical pixel on an output (and `WaylandSurfaceRenderElement` will do that for you).
Borders and focus rings should also have a width equal to an integer number of physical pixels to stay crisp (not to mention that `SolidColorRenderElement` does not anti-alias lines at fractional pixel positions).

Integer physical coordinates do not necessarily correspond to integer logical coordinates though.
Even with an integer scale = 2, a physical pixel at (1, 1) will be at the logical position of (0.5, 0.5).
This problem becomes much worse with fractional scale factors where most integer logical coordinates will fall on fractional physical coordinates.

Thus, niri uses fractional logical coordinates for most of its layout.
However, one needs to be very careful to keep things aligned to the physical grid to avoid artifacts like:

* Border width alternating 1 px thicker/thinner
* Border showing 1 px off from the window at certain positions
* 1 px gaps around rounded corners
* Slightly blurry window contents during resizes
* And so on...

The way it's handled in niri is:

1. All relevant sizes on a workspace are rounded to an integer physical coordinate according to the current output scale. Things like struts, gaps, border widths, working area location.

    It's important to understand that they remain fractional numbers in the logical space, but these numbers correspond to an integer number of pixels in the physical space.
    The rounding looks something like: `(logical_size * scale).round() / scale`.
    Whenever a workspace moves to an output with a different scale (or the output scale changes), all sizes are re-rounded from their original configured values to align with the new physical space.

2. The view offset and individual column/tile render offsets are *not* rounded to physical pixels, but:
3. `tiles_with_render_positions()` rounds tile positions to physical pixels as it returns them,
4. Custom shaders like opening, closing and resizing windows, are also careful to keep positions and sizes rounded to the physical pixels.

The idea is that every tile can assume that it is rendered at an integer physical coordinate, therefore when shifting the position by, say, border width (also rounded to integer physical coordinates), the new position will stay rounded to integer physical coordinates.
The same logic works for the rest of the layout thanks to gaps, struts and working area being similarly rounded.
This way, the entire layout is always aligned, as long as it is positioned at an integer physical coordinate (which rounding the tile positions effectively achieves).
