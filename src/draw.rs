//! Anti-aliased vector drawing for the custom chrome. GDI's RoundRect and
//! Ellipse are jagged; GDI+ gives smooth edges that match the DWM-rounded
//! popup menus. Text stays on GDI (ClearType already handles it).

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::GdiPlus::*;
use windows::core::BOOL;

/// Initialize GDI+ once per process. GDI+ keeps its own background thread:
/// suppressing it requires an output struct and manual notification hooks,
/// and with a null output GdiplusStartup fails outright.
pub fn init() {
    unsafe {
        let mut token = 0usize;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: BOOL(0),
            SuppressExternalCodecs: BOOL(1),
        };
        let _ = GdiplusStartup(&mut token, &input, std::ptr::null_mut());
    }
}

/// COLORREF (0x00BBGGRR) to GDI+ opaque ARGB.
#[inline]
pub fn argb(c: COLORREF) -> u32 {
    0xFF00_0000 | ((c.0 & 0xFF) << 16) | (c.0 & 0xFF00) | ((c.0 >> 16) & 0xFF)
}

/// An anti-aliased drawing context bound to an HDC. Create one per paint
/// pass; dropping it frees the GpGraphics. The source HDC is kept for
/// plain-GDI fallbacks.
pub struct Aa {
    g: *mut GpGraphics,
    hdc: HDC,
}

impl Aa {
    pub fn new(hdc: HDC) -> Self {
        unsafe {
            let mut g: *mut GpGraphics = std::ptr::null_mut();
            let _ = GdipCreateFromHDC(hdc, &mut g);
            if !g.is_null() {
                let _ = GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            }
            Self { g, hdc }
        }
    }

    pub fn ok(&self) -> bool {
        !self.g.is_null()
    }

    fn fallback_fill(&self, rc: &RECT, color: COLORREF) {
        unsafe {
            let b = CreateSolidBrush(color);
            FillRect(self.hdc, rc, b);
            let _ = DeleteObject(HGDIOBJ(b.0));
        }
    }

    /// Rounded-rectangle outline path; `top_only` rounds just the top
    /// corners. Caller frees it with GdipDeletePath.
    fn rounded_path(rc: &RECT, rad: i32, top_only: bool) -> *mut GpPath {
        unsafe {
            let mut path: *mut GpPath = std::ptr::null_mut();
            let _ = GdipCreatePath(FillModeAlternate, &mut path);
            if path.is_null() {
                return path;
            }
            let (l, t, r, b) = (rc.left, rc.top, rc.right, rc.bottom);
            let d = (rad.min((r - l) / 2).min((b - t) / 2).max(0)) * 2;
            if d <= 0 {
                let _ = GdipAddPathRectangleI(path, l, t, r - l, b - t);
            } else {
                let _ = GdipAddPathArcI(path, l, t, d, d, 180.0, 90.0);
                let _ = GdipAddPathArcI(path, r - d, t, d, d, 270.0, 90.0);
                if top_only {
                    let _ = GdipAddPathLineI(path, r, t + d / 2, r, b);
                    let _ = GdipAddPathLineI(path, r, b, l, b);
                } else {
                    let _ = GdipAddPathArcI(path, r - d, b - d, d, d, 0.0, 90.0);
                    let _ = GdipAddPathArcI(path, l, b - d, d, d, 90.0, 90.0);
                }
            }
            let _ = GdipClosePathFigure(path);
            path
        }
    }

    /// Filled rounded rectangle; `top_only` rounds just the top corners
    /// (selected-tab pill merging into the editor).
    pub fn fill_rounded(&self, rc: &RECT, rad: i32, top_only: bool, color: COLORREF) {
        if !self.ok() {
            // GDI fallback — better a square edge than nothing.
            self.fallback_fill(rc, color);
            return;
        }
        unsafe {
            let path = Self::rounded_path(rc, rad, top_only);
            if path.is_null() {
                self.fallback_fill(rc, color);
                return;
            }
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            let _ = GdipCreateSolidFill(argb(color), &mut brush);
            if !brush.is_null() {
                let _ = GdipFillPath(self.g, brush as *mut GpBrush, path);
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
            }
            let _ = GdipDeletePath(path);
        }
    }

    /// Rounded-rectangle outline, `w` pixels wide, drawn inside `rc`.
    pub fn stroke_rounded(&self, rc: &RECT, rad: i32, color: COLORREF, w: f32) {
        if !self.ok() {
            return;
        }
        unsafe {
            // Inset by half the pen so the stroke stays inside the rect.
            let half = (w / 2.0).ceil() as i32;
            let inner = RECT {
                left: rc.left + half,
                top: rc.top + half,
                right: rc.right - half,
                bottom: rc.bottom - half,
            };
            let path = Self::rounded_path(&inner, rad, false);
            if path.is_null() {
                return;
            }
            let mut pen: *mut GpPen = std::ptr::null_mut();
            let _ = GdipCreatePen1(argb(color), w, UnitPixel, &mut pen);
            if !pen.is_null() {
                let _ = GdipDrawPath(self.g, pen, path);
                let _ = GdipDeletePen(pen);
            }
            let _ = GdipDeletePath(path);
        }
    }

    pub fn fill_ellipse(&self, rc: &RECT, color: COLORREF) {
        if !self.ok() {
            self.fallback_fill(rc, color);
            return;
        }
        unsafe {
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            let _ = GdipCreateSolidFill(argb(color), &mut brush);
            if !brush.is_null() {
                let _ = GdipFillEllipseI(
                    self.g,
                    brush as *mut GpBrush,
                    rc.left,
                    rc.top,
                    rc.right - rc.left,
                    rc.bottom - rc.top,
                );
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
            }
        }
    }

    /// Anti-aliased polyline through `pts` (2+ points), `w` in pixels.
    pub fn stroke(&self, pts: &[(i32, i32)], color: COLORREF, w: f32) {
        if !self.ok() || pts.len() < 2 {
            return;
        }
        unsafe {
            let mut pen: *mut GpPen = std::ptr::null_mut();
            let _ = GdipCreatePen1(argb(color), w, UnitPixel, &mut pen);
            if pen.is_null() {
                return;
            }
            let points: Vec<Point> = pts.iter().map(|p| Point { X: p.0, Y: p.1 }).collect();
            let _ = GdipDrawLinesI(self.g, pen, points.as_ptr(), points.len() as i32);
            let _ = GdipDeletePen(pen);
        }
    }
}

impl Drop for Aa {
    fn drop(&mut self) {
        unsafe {
            if !self.g.is_null() {
                let _ = GdipDeleteGraphics(self.g);
            }
        }
    }
}
