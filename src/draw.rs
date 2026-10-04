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

/// Fills `rc` with a solid color through the DC's stock brush: no brush
/// object is created or destroyed.
pub fn fill(hdc: HDC, rc: &RECT, color: COLORREF) {
    unsafe {
        SetDCBrushColor(hdc, color);
        FillRect(hdc, rc, HBRUSH(GetStockObject(DC_BRUSH).0));
    }
}

/// Anti-aliased shapes on an HDC that GDI also draws on. A GDI+ Graphics
/// changes its HDC's state while it exists (GDI text drawn meanwhile loses
/// the back buffer's viewport offset), so each shape gets its own short-lived
/// Graphics and GDI always sees an untouched DC. Paints that draw no shapes
/// never touch GDI+ at all.
pub struct Aa {
    hdc: HDC,
}

impl Aa {
    pub fn new(hdc: HDC) -> Self {
        Self { hdc }
    }

    /// Runs `f` with a fresh anti-aliasing Graphics; false if GDI+ is
    /// unavailable. GDI+ follows the HDC's viewport origin itself.
    fn with_g(&self, f: impl FnOnce(*mut GpGraphics)) -> bool {
        unsafe {
            let mut g: *mut GpGraphics = std::ptr::null_mut();
            let _ = GdipCreateFromHDC(self.hdc, &mut g);
            if g.is_null() {
                return false;
            }
            let _ = GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
            f(g);
            let _ = GdipDeleteGraphics(g);
            true
        }
    }

    fn fallback_fill(&self, rc: &RECT, color: COLORREF) {
        fill(self.hdc, rc, color);
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
        let drawn = self.with_g(|g| unsafe {
            let path = Self::rounded_path(rc, rad, top_only);
            if path.is_null() {
                return;
            }
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            let _ = GdipCreateSolidFill(argb(color), &mut brush);
            if !brush.is_null() {
                let _ = GdipFillPath(g, brush as *mut GpBrush, path);
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
            }
            let _ = GdipDeletePath(path);
        });
        if !drawn {
            // GDI fallback — better a square edge than nothing.
            self.fallback_fill(rc, color);
        }
    }

    /// Rounded-rectangle outline, `w` pixels wide, drawn inside `rc`.
    pub fn stroke_rounded(&self, rc: &RECT, rad: i32, color: COLORREF, w: f32) {
        self.with_g(|g| unsafe {
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
                let _ = GdipDrawPath(g, pen, path);
                let _ = GdipDeletePen(pen);
            }
            let _ = GdipDeletePath(path);
        });
    }

    pub fn fill_ellipse(&self, rc: &RECT, color: COLORREF) {
        let drawn = self.with_g(|g| unsafe {
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            let _ = GdipCreateSolidFill(argb(color), &mut brush);
            if !brush.is_null() {
                let _ = GdipFillEllipseI(
                    g,
                    brush as *mut GpBrush,
                    rc.left,
                    rc.top,
                    rc.right - rc.left,
                    rc.bottom - rc.top,
                );
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
            }
        });
        if !drawn {
            self.fallback_fill(rc, color);
        }
    }

    /// Anti-aliased polyline through `pts` (2+ points), `w` in pixels.
    pub fn stroke(&self, pts: &[(i32, i32)], color: COLORREF, w: f32) {
        if pts.len() < 2 {
            return;
        }
        self.with_g(|g| unsafe {
            let mut pen: *mut GpPen = std::ptr::null_mut();
            let _ = GdipCreatePen1(argb(color), w, UnitPixel, &mut pen);
            if pen.is_null() {
                return;
            }
            let mut points = [Point::default(); 8];
            let n = pts.len().min(points.len());
            for (d, p) in points.iter_mut().zip(pts) {
                *d = Point { X: p.0, Y: p.1 };
            }
            let _ = GdipDrawLinesI(g, pen, points.as_ptr(), n as i32);
            let _ = GdipDeletePen(pen);
        });
    }
}
