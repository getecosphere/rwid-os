//! Screen capture (`ext-image-copy-capture-v1`) for screen recording: OBS and the
//! portal (xdg-desktop-portal-wlr) ask for frames of an output; the udev backend
//! renders that output once more into an offscreen texture and copies the
//! pixels into the client's shared-memory buffer. SHM only (no DMA-BUF), the
//! cursor is not drawn into the frame.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, ExportMem, Offscreen, Renderer, TextureMapping,
            damage::OutputDamageTracker, element::RenderElement, gles::GlesTexture,
        },
    },
    output::Output,
    reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_shm},
    utils::{Buffer as BufferCoords, Rectangle, Size, Transform},
    wayland::shm::with_buffer_contents_mut,
};

/// Render `elements` of `output` into the SHM `buffer` (ARGB8888 / XRGB8888).
pub fn render_into_shm<R, E>(
    renderer: &mut R,
    output: &Output,
    elements: &[E],
    clear: Color32F,
    buffer: &WlBuffer,
) -> Result<(), String>
where
    R: Renderer + Offscreen<GlesTexture> + Bind<GlesTexture> + ExportMem,
    E: RenderElement<R>,
{
    let mode = output.current_mode().ok_or("output without mode")?;
    let size: Size<i32, BufferCoords> = mode.size.to_logical(1).to_buffer(1, Transform::Normal);
    let mut texture = renderer
        .create_buffer(Fourcc::Abgr8888, size)
        .map_err(|e| format!("offscreen buffer: {e:?}"))?;
    let mapping = {
        let mut fb = renderer
            .bind(&mut texture)
            .map_err(|e| format!("bind: {e:?}"))?;
        let mut damage = OutputDamageTracker::new(
            mode.size,
            output.current_scale().fractional_scale(),
            Transform::Normal,
        );
        damage
            .render_output(renderer, &mut fb, 0, elements, clear)
            .map_err(|e| format!("render: {e:?}"))?;
        renderer
            .copy_framebuffer(&fb, Rectangle::from_size(size), Fourcc::Abgr8888)
            .map_err(|e| format!("read back: {e:?}"))?
    };
    // GlesRenderer draws into an offscreen texture already y-flipped (so it samples upright);
    // the read-back rows therefore come out top-down although the mapping reports `flipped()`.
    // Verified with grim on the udev backend: honouring `flipped()` gave an upside-down frame.
    let flipped = !mapping.flipped();
    let pixels = renderer
        .map_texture(&mapping)
        .map_err(|e| format!("map: {e:?}"))?;
    copy_rgba_to_shm(pixels, size.w as usize, size.h as usize, flipped, buffer)
}

/// Copy tightly packed RGBA rows into an ARGB/XRGB SHM buffer (BGRA in memory).
fn copy_rgba_to_shm(
    src: &[u8],
    w: usize,
    h: usize,
    flipped: bool,
    buffer: &WlBuffer,
) -> Result<(), String> {
    with_buffer_contents_mut(buffer, |ptr, len, meta| {
        let alpha = match meta.format {
            wl_shm::Format::Argb8888 => false,
            wl_shm::Format::Xrgb8888 => true,
            other => return Err(format!("unsupported format {other:?}")),
        };
        if meta.width as usize != w || meta.height as usize != h {
            return Err("buffer size does not match the output".into());
        }
        let (stride, offset) = (meta.stride as usize, meta.offset as usize);
        if offset + stride * h > len || src.len() < w * h * 4 || stride < w * 4 {
            return Err("buffer too small".into());
        }
        // SAFETY: bounds checked above; the pool is mapped for `len` bytes.
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr.add(offset), stride * h) };
        convert_rows(src, dst, w, h, stride, flipped, alpha);
        Ok(())
    })
    .map_err(|e| format!("shm: {e:?}"))?
}

/// RGBA → BGRA, row by row (optionally bottom-up); `opaque` forces alpha 255.
fn convert_rows(
    src: &[u8],
    dst: &mut [u8],
    w: usize,
    h: usize,
    stride: usize,
    flipped: bool,
    opaque: bool,
) {
    for y in 0..h {
        let sy = if flipped { h - 1 - y } else { y };
        let s = &src[sy * w * 4..(sy + 1) * w * 4];
        let d = &mut dst[y * stride..y * stride + w * 4];
        for (sp, dp) in s
            .as_chunks::<4>()
            .0
            .iter()
            .zip(d.as_chunks_mut::<4>().0.iter_mut())
        {
            dp[0] = sp[2];
            dp[1] = sp[1];
            dp[2] = sp[0];
            dp[3] = if opaque { 255 } else { sp[3] };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::convert_rows;

    #[test]
    fn rgba_becomes_bgra_with_stride_and_flip() {
        // 2×2: red, green / blue, white (RGBA)
        let src = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
        ];
        let mut dst = vec![0u8; 12 * 2]; // stride 12 (padding)
        convert_rows(&src, &mut dst, 2, 2, 12, false, false);
        assert_eq!(&dst[0..8], &[0, 0, 255, 255, 0, 255, 0, 255]);
        assert_eq!(&dst[12..20], &[255, 0, 0, 255, 255, 255, 255, 128]);
        // flipped + opaque: bottom row first, alpha forced
        convert_rows(&src, &mut dst, 2, 2, 12, true, true);
        assert_eq!(&dst[0..8], &[255, 0, 0, 255, 255, 255, 255, 255]);
    }
}
