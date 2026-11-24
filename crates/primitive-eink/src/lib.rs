//! Copied from Primitive Browser 0.2.12 (`crates/kb-kindle/src/eink.rs`), with `Panel::turn` added.
//!
//! Drawing to the e-ink panel in-process: the pixels are copied into the framebuffer and the update
//! ioctl is sent from here, as the Kindle's own reader does, instead of starting fbink for every update.
//!
//! Two reasons, both measured on the Oasis 3 with the kernel's e-ink debug log:
//!
//! - Starting fbink costs about 170 ms an update (process start, device detection, BMP decode). With the
//!   frame file and the rest, a page turn took 280-1340 ms from button press to update; the stock reader
//!   takes 60-100 ms.
//! - For AUTO, fbink tells the driver to use GC16, the flashing waveform, for any update with gray in it
//!   (`hist_gray_waveform_mode`). Anti-aliased text always has gray, so every page turn flashed black (white
//!   in dark mode). Here gray content gets REAGL, the non-flashing page-turn waveform.
//!
//! The structure and ioctl are the Oasis 3 (Zelda platform) ones, from FBInk's `eink/mxcfb-kindle.h`.

pub mod display;
pub use display::Display;

use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::io::AsRawFd,
};

const FRAMEBUFFER: &str = "/dev/fb0";

/// Waveform codes on the Zelda platform (Oasis 2 and 3). The kernel log prints these numbers.
pub const WAVE_DU: u32 = 1;
pub const WAVE_GC16: u32 = 2;
#[allow(dead_code)]
pub const WAVE_GL16: u32 = 3;
#[allow(dead_code)]
pub const WAVE_REAGL: u32 = 4;
pub const WAVE_AUTO: u32 = 257;

/// The waveform code for a name from the Display sheet.
#[allow(dead_code)]
pub fn wave_code(name: &str) -> Option<u32> {
    match name {
        "AUTO" => Some(WAVE_AUTO),
        "DU" => Some(WAVE_DU),
        "GC16" => Some(WAVE_GC16),
        "GL16" => Some(WAVE_GL16),
        "REAGL" => Some(WAVE_REAGL),
        _ => None,
    }
}

/// The waveforms the driver picks between for an AUTO update: one for pure black and white content,
/// one for content with gray. Only AUTO reads them. REAGL asks for REAGL in both, as fbink does.
fn hist_waves(wave: u32) -> (u32, u32) {
    if wave == WAVE_REAGL {
        (WAVE_REAGL, WAVE_REAGL)
    } else {
        (WAVE_DU, WAVE_REAGL)
    }
}

const UPDATE_MODE_PARTIAL: u32 = 0;
const UPDATE_MODE_FULL: u32 = 1;
/// Let the driver read the panel's temperature itself (the kernel log shows `temp=4096`).
const TEMP_USE_AMBIENT: i32 = 0x1000;

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
struct Rect {
    top: u32,
    left: u32,
    width: u32,
    height: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
struct AltBuffer {
    phys_addr: u32,
    width: u32,
    height: u32,
    region: Rect,
}

/// `struct mxcfb_update_data_zelda`.
#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
struct UpdateZelda {
    region: Rect,
    waveform_mode: u32,
    update_mode: u32,
    update_marker: u32,
    temp: i32,
    flags: u32,
    dither_mode: i32,
    quant_bit: i32,
    alt_buffer: AltBuffer,
    hist_bw_waveform_mode: u32,
    hist_gray_waveform_mode: u32,
    ts_pxp: u32,
    ts_epdc: u32,
}

/// `_IOW(kind, nr, size)` on Linux.
const fn iow(kind: u8, nr: u8, size: usize) -> u32 {
    (1u32 << 30) | ((size as u32) << 16) | ((kind as u32) << 8) | nr as u32
}

/// `MXCFB_SEND_UPDATE_ZELDA = _IOW('F', 0x2E, struct mxcfb_update_data_zelda)`.
const MXCFB_SEND_UPDATE_ZELDA: u32 = iow(b'F', 0x2E, std::mem::size_of::<UpdateZelda>());
const FBIOGET_VSCREENINFO: u32 = 0x4600;
const FBIOGET_FSCREENINFO: u32 = 0x4602;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Bitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

/// `struct fb_var_screeninfo`.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct VarInfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: Bitfield,
    green: Bitfield,
    blue: Bitfield,
    transp: Bitfield,
    nonstd: u32,
    activate: u32,
    height: u32,
    width: u32,
    accel_flags: u32,
    pixclock: u32,
    left_margin: u32,
    right_margin: u32,
    upper_margin: u32,
    lower_margin: u32,
    hsync_len: u32,
    vsync_len: u32,
    sync: u32,
    vmode: u32,
    rotate: u32,
    colorspace: u32,
    reserved: [u32; 4],
}

/// `struct fb_fix_screeninfo`.
#[repr(C)]
#[derive(Clone, Copy)]
struct FixInfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    kind: u32,
    kind_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: libc::c_ulong,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

/// Observed geometry of a supported, mapped portrait framebuffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramebufferGeometry {
    pub native_size: (u32, u32),
    pub turn: u32,
    pub yoffset: u32,
    pub pitch: u32,
}

type Region = (u32, u32, u32, u32);

/// The fixed layout used to establish a panel mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mapping {
    start: libc::c_ulong,
    len: usize,
    pitch: usize,
}

impl From<FixInfo> for Mapping {
    fn from(fix: FixInfo) -> Self {
        Self {
            start: fix.smem_start,
            len: fix.smem_len as usize,
            pitch: fix.line_length as usize,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VisibleRows {
    width: u32,
    height: u32,
    pitch: usize,
    offset: usize,
}

impl VisibleRows {
    fn new(var: VarInfo, pitch: usize) -> Option<Self> {
        let width = var.xres;
        let height = var.yres;
        if width == 0 || height == 0 || width as usize > pitch {
            return None;
        }
        let offset = (var.yoffset as usize).checked_mul(pitch)?;
        Some(Self {
            width,
            height,
            pitch,
            offset,
        })
    }

    /// Each tightly packed source row and its start in the mapped framebuffer.
    fn runs(
        self,
        (x, y, w, h): Region,
        source_len: usize,
        mapped_len: usize,
    ) -> Option<impl Iterator<Item = (std::ops::Range<usize>, usize)>> {
        let (right, bottom) = (x.checked_add(w)?, y.checked_add(h)?);
        if w == 0 || h == 0 || right > self.width || bottom > self.height {
            return None;
        }
        let x = x as usize;
        let right = right as usize;
        let stride = self.width as usize;
        let last_row = (bottom - 1) as usize;
        let source_end = (bottom as usize).checked_mul(stride)?;
        let mapped_end = self
            .offset
            .checked_add(last_row.checked_mul(self.pitch)?)?
            .checked_add(right)?;
        if source_end > source_len || mapped_end > mapped_len {
            return None;
        }
        Some((y as usize..bottom as usize).map(move |row| {
            let source = row * stride;
            (
                source + x..source + right,
                self.offset + row * self.pitch + x,
            )
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PanelConfig {
    rotation: u32,
    rows: VisibleRows,
}

impl PanelConfig {
    fn geometry(self) -> FramebufferGeometry {
        FramebufferGeometry {
            native_size: (self.rows.width, self.rows.height),
            turn: self.rotation,
            // Offset is built from a kernel u32 row offset and this pitch.
            yoffset: (self.rows.offset / self.rows.pitch) as u32,
            pitch: self.rows.pitch as u32,
        }
    }

    fn new(
        var: VarInfo,
        opened: Mapping,
        current: Mapping,
        (width, height): (u32, u32),
    ) -> Option<Self> {
        if opened != current
            || var.bits_per_pixel != 8
            || var.xoffset != 0
            || !matches!(var.rotate, 0 | 2)
            || (var.xres, var.yres) != (width, height)
        {
            return None;
        }
        // A rejected viewport must never write to the old mapping, including when the
        // firmware has moved the visible page beyond its end.
        let rows = VisibleRows::new(var, opened.pitch)?;
        let frame_len = (width as usize).checked_mul(height as usize)?;
        let _ = rows.runs((0, 0, width, height), frame_len, opened.len)?;
        Some(Self {
            rotation: var.rotate,
            rows,
        })
    }
}

/// The framebuffer, mapped for writing, and the update counter.
pub struct Panel {
    file: File,
    map: *mut u8,
    mapping: Mapping,
    marker: u32,
}

impl Panel {
    /// Open and map the framebuffer. Fails off the Kindle, or when the framebuffer is not 8-bit gray.
    pub fn open() -> Result<Panel, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(FRAMEBUFFER)
            .map_err(|e| format!("open {FRAMEBUFFER}: {e}"))?;
        let fd = file.as_raw_fd();
        let mapping = fixed_info(fd)?.into();
        let var = var_info(fd)?;
        if var.bits_per_pixel != 8 {
            return Err(format!(
                "framebuffer is {} bits per pixel, not 8",
                var.bits_per_pixel
            ));
        }
        let Mapping { len, .. } = mapping;
        // SAFETY: mapping the framebuffer this process opened; the length is the driver's own.
        let map = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if map == libc::MAP_FAILED {
            return Err(format!("mmap: {}", io::Error::last_os_error()));
        }
        Ok(Panel {
            file,
            map: map as *mut u8,
            mapping,
            marker: 0,
        })
    }

    /// Copy the region (`x`, `y`, `w`, `h`) of `frame` (a whole screen of `width` gray pixels a row) to the
    /// framebuffer and send the update for that region only, with `wave`; `full` drives every pixel of it (a
    /// flash with GC16). An upside-down screen (rotation 2) is drawn as it is: the panel shows the memory
    /// unturned. Returns `Ok(false)` without drawing when this path cannot draw the screen as it is
    /// (landscape, or another size), so the caller can use fbink.
    pub fn draw(
        &mut self,
        width: u32,
        region: (u32, u32, u32, u32),
        frame: &[u8],
        wave: u32,
        full: bool,
    ) -> Result<bool, String> {
        let var = var_info(self.file.as_raw_fd())?;
        if !matches!(var.rotate, 0 | 2) || var.xoffset != 0 || var.xres != width {
            return Ok(false);
        }
        let Some(rows) = VisibleRows::new(var, self.mapping.pitch) else {
            return Ok(false);
        };
        self.draw_rows(rows, region, frame, wave, full)
    }

    fn viewport_config(&self, size: (u32, u32)) -> Result<Option<PanelConfig>, String> {
        let fd = self.file.as_raw_fd();
        let current = fixed_info(fd)?.into();
        Ok(PanelConfig::new(var_info(fd)?, self.mapping, current, size))
    }

    fn matches_viewport(&self, expected: PanelConfig) -> Result<bool, String> {
        let size = (expected.rows.width, expected.rows.height);
        Ok(self.viewport_config(size)? == Some(expected))
    }

    fn draw_view(
        &mut self,
        expected: PanelConfig,
        region: Region,
        frame: &[u8],
        wave: u32,
        full: bool,
    ) -> Result<bool, String> {
        if !self.matches_viewport(expected)? {
            return Ok(false);
        }
        self.draw_rows(expected.rows, region, frame, wave, full)
    }

    fn draw_rows(
        &mut self,
        rows: VisibleRows,
        region: Region,
        frame: &[u8],
        wave: u32,
        full: bool,
    ) -> Result<bool, String> {
        let Some(runs) = rows.runs(region, frame.len(), self.mapping.len) else {
            return Ok(false);
        };
        for (source, destination) in runs {
            let row = &frame[source];
            // SAFETY: the runs were checked against the lengths of the source and the mapping.
            unsafe {
                std::ptr::copy_nonoverlapping(row.as_ptr(), self.map.add(destination), row.len())
            };
        }
        let (x, y, width, height) = region;
        self.marker = self.marker.wrapping_add(1).max(1);
        let (hist_bw, hist_gray) = hist_waves(wave);
        let mut update = UpdateZelda {
            region: Rect {
                top: y,
                left: x,
                width,
                height,
            },
            waveform_mode: wave,
            update_mode: if full {
                UPDATE_MODE_FULL
            } else {
                UPDATE_MODE_PARTIAL
            },
            update_marker: self.marker,
            temp: TEMP_USE_AMBIENT,
            hist_bw_waveform_mode: hist_bw,
            hist_gray_waveform_mode: hist_gray,
            ..UpdateZelda::default()
        };
        // SAFETY: the driver reads an UpdateZelda; the ioctl number carries its size.
        if unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                MXCFB_SEND_UPDATE_ZELDA as _,
                &mut update,
            )
        } < 0
        {
            return Err(format!("MXCFB_SEND_UPDATE: {}", io::Error::last_os_error()));
        }
        Ok(true)
    }
}

impl Panel {
    fn snapshot_viewport(&self, expected: FramebufferGeometry) -> Option<Vec<u8>> {
        let config = self.viewport_config(expected.native_size).ok()??;
        if config.geometry() != expected {
            return None;
        }
        let (width, height) = expected.native_size;
        let frame_len = (width as usize).checked_mul(height as usize)?;
        let mut frame = vec![0; frame_len];
        for (destination, source) in
            config
                .rows
                .runs((0, 0, width, height), frame_len, self.mapping.len)?
        {
            let row = &mut frame[destination];
            // SAFETY: the ranges were checked against the frame and mapping lengths.
            unsafe {
                std::ptr::copy_nonoverlapping(self.map.add(source), row.as_mut_ptr(), row.len())
            };
        }
        self.matches_viewport(config).ok()?.then_some(frame)
    }

    /// The visible page as it is in memory (panel orientation), `width` bytes a row for `height` rows: what is on the
    /// screen now, so it can be put back later. `None` when the page does not fit the mapping.
    pub fn snapshot(&self, width: u32, height: u32) -> Option<Vec<u8>> {
        let var = var_info(self.file.as_raw_fd()).ok()?;
        let (w, h) = (width as usize, height as usize);
        let Mapping { len, pitch, .. } = self.mapping;
        let base = var.yoffset as usize * pitch;
        if h == 0 || w > pitch || base + (h - 1) * pitch + w > len {
            return None;
        }
        let mut out = vec![0u8; w * h];
        for r in 0..h {
            // SAFETY: the source run is inside the mapping (checked above) and the destination inside `out`.
            unsafe { std::ptr::copy_nonoverlapping(self.map.add(base + r * pitch), out.as_mut_ptr().add(r * w), w) };
        }
        Some(out)
    }

    /// The framebuffer's own rotation (0 upright, 2 upside down, 1 and 3 landscape).
    pub fn turn(&self) -> u32 {
        var_info(self.file.as_raw_fd()).map_or(0, |v| v.rotate)
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        // SAFETY: unmapping the mapping made in `open`.
        unsafe { libc::munmap(self.map as *mut libc::c_void, self.mapping.len) };
    }
}

fn fixed_info(fd: i32) -> Result<FixInfo, String> {
    // SAFETY: FixInfo is plain data; the ioctl fills it.
    let mut info: FixInfo = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut info) } < 0 {
        return Err(format!(
            "FBIOGET_FSCREENINFO: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(info)
}

fn var_info(fd: i32) -> Result<VarInfo, String> {
    let mut var = VarInfo::default();
    // SAFETY: VarInfo is plain data; the ioctl fills it.
    if unsafe { libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut var) } < 0 {
        return Err(format!("FBIOGET_VSCREENINFO: {}", io::Error::last_os_error()));
    }
    Ok(var)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_viewport_configurations_cannot_reach_the_framebuffer_copy() {
        let size = (1264, 1680);
        let var = VarInfo {
            xres: size.0,
            yres: size.1,
            yoffset: 0,
            bits_per_pixel: 8,
            ..VarInfo::default()
        };
        let mapping = Mapping {
            start: 0x5000,
            len: 4_587_520,
            pitch: 1280,
        };
        assert!(PanelConfig::new(var, mapping, mapping, size).is_some());
        assert!(PanelConfig::new(VarInfo { rotate: 2, ..var }, mapping, mapping, size).is_some());
        for bad in [
            VarInfo { rotate: 1, ..var },
            VarInfo { rotate: 3, ..var },
            VarInfo {
                bits_per_pixel: 16,
                ..var
            },
            VarInfo { xoffset: 1, ..var },
            VarInfo {
                yoffset: u32::MAX,
                ..var
            },
            VarInfo {
                xres: size.1,
                yres: size.0,
                ..var
            },
            VarInfo {
                yres: size.1 - 1,
                ..var
            },
        ] {
            assert!(PanelConfig::new(bad, mapping, mapping, size).is_none());
        }
        for changed in [
            Mapping {
                start: mapping.start + 4,
                ..mapping
            },
            Mapping {
                len: mapping.len - 1,
                ..mapping
            },
            Mapping {
                pitch: mapping.pitch + 16,
                ..mapping
            },
        ] {
            assert!(PanelConfig::new(var, mapping, changed, size).is_none());
        }
        let cramped = Mapping {
            pitch: 1260,
            ..mapping
        };
        assert!(PanelConfig::new(var, cramped, cramped, size).is_none());
    }

    #[test]
    fn update_struct_matches_the_zelda_header() {
        // 4 (rect) + 7 + 7 (alt buffer: 3 + rect) + 4 words.
        assert_eq!(std::mem::size_of::<UpdateZelda>(), 88);
        assert_eq!(MXCFB_SEND_UPDATE_ZELDA, 0x4058_462E);
        assert_eq!(std::mem::size_of::<VarInfo>(), 160);
    }

    #[test]
    fn auto_asks_for_reagl_on_gray_not_gc16() {
        assert_eq!(hist_waves(WAVE_AUTO), (WAVE_DU, WAVE_REAGL));
        assert_eq!(hist_waves(WAVE_REAGL), (WAVE_REAGL, WAVE_REAGL));
        assert_eq!(wave_code("AUTO"), Some(257));
        assert_eq!(wave_code("GL16"), Some(3));
        assert_eq!(wave_code("nope"), None);
    }
}
