//! The content for exporting the canvas to a file (PNG / PDF).
//!
//! It has no menus or dialogs (it is called from the tray menu). Like the image sent to the AI,
//! it draws the sheet's image (under the strokes) and the strokes on white. It does not include the grid

use std::path::{Path, PathBuf};

use crate::canvas::{Canvas, Snapshot};
use crate::color;
use crate::pdf::{self, PdfPage};
use crate::protocol::{ImagePlacement, PageSize, Sheet, Stroke};
use crate::render::{self, Rect, unpremultiplied_rgba};

/// Export resolution: 1 CSS pixel = 2 pixels
pub const EXPORT_SCALE: f64 = 2.0;
/// The upper limit (in pixels) of the longer side in a PNG export. Larger areas use a lower scale
pub const MAX_EXPORT_SIDE: f64 = 4096.0;
/// The size (in pt) of 1 CSS pixel in a PDF. CSS 96px = 1in, PDF 72pt = 1in
pub const PT_PER_CSS_PIXEL: f64 = 0.75;

/// The scale for a PNG export. Only when the longer side exceeds MAX_EXPORT_SIDE pixels at the
/// default scale of 2 does it lower the scale so the longer side becomes MAX_EXPORT_SIDE pixels
pub fn export_scale(area: Rect) -> f64 {
    EXPORT_SCALE.min(MAX_EXPORT_SIDE / area.width.max(area.height))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Pdf,
}

/// The sheet to export
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    CurrentSheet,
    AllSheets,
}

impl Format {
    /// Extension (without the dot): "png" / "pdf"
    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Pdf => "pdf",
        }
    }
}

/// Fit the area inside the sheet (the sheet starts at (0, 0)). None if they do not overlap
pub fn clamp_to_page(area: Rect, page: PageSize) -> Option<Rect> {
    area.intersect(Rect::from(page))
}

/// The default file name. stamp is "YYYY-MM-DD-HHMM" (local time; the caller gets it from the OS,
/// and if it cannot get it, it is None and the time is omitted from the name). sheet is the number
/// (from 1) of the sheet when exporting a single sheet. None for all sheets
pub fn default_file_name(stamp: Option<&str>, sheet: Option<usize>, format: Format) -> String {
    let mut name = String::from("draw-mcp");
    if let Some(stamp) = stamp {
        name.push('-');
        name.push_str(stamp);
    }
    if let Some(sheet) = sheet {
        name.push_str(&format!("-sheet-{sheet}"));
    }
    name.push_str(&format!(".{}", format.extension()));
    name
}

/// The path with format's extension added when the extension is missing or different (case-insensitive)
pub fn with_extension(path: &Path, format: Format) -> PathBuf {
    let has_extension = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case(format.extension()));
    if has_extension {
        path.to_path_buf()
    } else {
        let mut with = path.as_os_str().to_os_string();
        with.push(format!(".{}", format.extension()));
        PathBuf::from(with)
    }
}

/// Turn a sheet's area into a PNG (at a scale that does not exceed MAX_EXPORT_SIDE pixels on the
/// longer side). Even if the sheet has an image placement, if its bytes are not there yet, the
/// image is not drawn (same as the MCP image)
fn sheet_png(canvas: &Canvas, sheet: &Sheet, area: Rect) -> Result<Vec<u8>, String> {
    let stored = sheet.image.as_ref().and_then(|placement| {
        canvas
            .image_data(&placement.id)
            .map(|data| (placement, data))
    });
    let image = stored
        .as_ref()
        .map(|(placement, data)| (*placement, data.as_ref()));
    let pixels = render::render_area(&sheet.strokes, image, area, export_scale(area));
    render::encode_png(&pixels).map_err(|error| error.to_string())
}

/// Turn a sheet's area into one PDF page (the page size is the area's size). Strokes are written
/// by the content as vectors. An imported image is pasted as the original image (color and alpha),
/// not as drawn pixels
fn pdf_page(canvas: &Canvas, sheet: &Sheet, area: Rect) -> Result<PdfPage, String> {
    // A placement whose bytes are not there yet or cannot be read is not pasted (same as PNG)
    let (placement, image) = match sheet.image.as_ref() {
        Some(placement) => match canvas.image_data(&placement.id) {
            Some(stored) => match unpremultiplied_rgba(&stored) {
                Some((width, height, rgba)) => (
                    Some(placement),
                    Some(pdf::image_from_rgba(width, height, &rgba)?),
                ),
                None => (None, None),
            },
            None => (None, None),
        },
        None => (None, None),
    };
    Ok(PdfPage {
        width_pt: area.width * PT_PER_CSS_PIXEL,
        height_pt: area.height * PT_PER_CSS_PIXEL,
        content: page_content(area, &sheet.strokes, placement),
        image,
    })
}

/// Build the content (the Contents stream) of a PDF page, in the order: cut by the area → map the
/// canvas coordinates to the page → image (under the strokes) → strokes. Strokes are not drawn
/// pixels; they are written as lines with the same points (`smooth_points`) and width (`radius_at`)
/// as the page's drawing
fn page_content(area: Rect, strokes: &[Stroke], image: Option<&ImagePlacement>) -> Vec<u8> {
    let mut content = String::new();
    let (width, height) = (
        pdf::pt(area.width * PT_PER_CSS_PIXEL),
        pdf::pt(area.height * PT_PER_CSS_PIXEL),
    );
    // 1. Cut everything outside the page (outside the area)
    content.push_str(&format!("0 0 {width} {height} re W n\n"));
    // 2. Map the canvas coordinates (CSS pixels, y down) to the page (pt, y up)
    content.push_str(&format!(
        "{} 0 0 -{} {} {} cm\n",
        pdf::pt(PT_PER_CSS_PIXEL),
        pdf::pt(PT_PER_CSS_PIXEL),
        pdf::pt(-area.x * PT_PER_CSS_PIXEL),
        pdf::pt((area.y + area.height) * PT_PER_CSS_PIXEL)
    ));
    // 3. Image (the placement is the same as on the canvas. The coordinates have y flipped, so to
    //    keep top and bottom from being reversed, multiply the height by a negative and put the
    //    origin at the bottom edge)
    if let Some(image) = image {
        content.push_str(&format!(
            "q {} 0 0 -{} {} {} cm /Im0 Do Q\n",
            number(image.width),
            number(image.height),
            number(image.x),
            number(image.y + image.height)
        ));
    }
    // 4. Strokes (in stacking order)
    for stroke in strokes {
        // Ones with a different color shape (a data error) are drawn black (same as the page's drawing)
        let [red, green, blue] = color::parse(&stroke.color).unwrap_or([0, 0, 0]);
        content.push_str(&format!(
            "{:.3} {:.3} {:.3} RG 1 J 1 j\n",
            f64::from(red) / 255.0,
            f64::from(green) / 255.0,
            f64::from(blue) / 255.0
        ));
        let points = render::smooth_points(&stroke.points);
        match points.as_slice() {
            [] => {}
            [only] => {
                // With only one point, a zero-length line to the same point (a circle with round caps)
                let (x, y) = (number(only.x), number(only.y));
                content.push_str(&format!(
                    "{} w {} {} m {} {} l S\n",
                    number(render::radius_at(only.pressure) * 2.0),
                    x,
                    y,
                    x,
                    y
                ));
            }
            _ => {
                for pair in points.windows(2) {
                    // The width is the average of the two points' radii × 2 (= the sum of the radii)
                    let width =
                        render::radius_at(pair[0].pressure) + render::radius_at(pair[1].pressure);
                    content.push_str(&format!(
                        "{} w {} {} m {} {} l S\n",
                        number(width),
                        number(pair[0].x),
                        number(pair[0].y),
                        number(pair[1].x),
                        number(pair[1].y)
                    ));
                }
            }
        }
    }
    content.into_bytes()
}

/// A number in the content (canvas coordinates). Up to 2 decimal places, dropping trailing zeros and the point
fn number(value: f64) -> String {
    let text = format!("{value:.2}");
    let trimmed = text.trim_end_matches('0');
    trimmed.strip_suffix('.').unwrap_or(trimmed).to_string()
}

/// Look up a sheet by ID
fn find_sheet<'a>(snapshot: &'a Snapshot, id: &str) -> Result<&'a Sheet, String> {
    snapshot
        .sheets
        .iter()
        .find(|sheet| sheet.id == id)
        .ok_or_else(|| "The sheet was deleted.".to_string())
}

/// Turn the area (the area after fitting it inside the sheet) of the sheet given by ID into a PNG
pub fn sheet_png_by_id(canvas: &Canvas, sheet: &str, area: Rect) -> Result<Vec<u8>, String> {
    let snapshot = canvas.snapshot();
    sheet_png(canvas, find_sheet(&snapshot, sheet)?, area)
}

/// Build a PDF. CurrentSheet makes the area of the sheet given by ID one page; AllSheets makes one
/// page per sheet, in sheet order, each covering the whole `page` (area is not used)
pub fn pdf(
    canvas: &Canvas,
    target: Target,
    sheet: &str,
    area: Rect,
    page: PageSize,
) -> Result<Vec<u8>, String> {
    let snapshot = canvas.snapshot();
    let pages = match target {
        Target::CurrentSheet => vec![pdf_page(canvas, find_sheet(&snapshot, sheet)?, area)?],
        Target::AllSheets => snapshot
            .sheets
            .iter()
            .map(|sheet| pdf_page(canvas, sheet, Rect::from(page)))
            .collect::<Result<_, _>>()?,
    };
    Ok(crate::pdf::write_pdf(&pages))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Author, CanvasOp, INITIAL_SHEET_ID, Stroke, StrokePoint};
    use std::path::PathBuf;

    /// A single-point stroke at canvas coordinates (x, y)
    fn stroke(id: &str, x: f64, y: f64) -> Stroke {
        Stroke {
            id: id.into(),
            author: Author::Human,
            color: crate::color::BLACK.into(),
            points: vec![StrokePoint {
                x,
                y,
                pressure: 1.0,
            }],
        }
    }

    /// A canvas with strokes placed on the current sheet
    fn canvas_with_strokes(strokes: Vec<Stroke>) -> Canvas {
        let canvas = Canvas::new();
        for (index, stroke) in strokes.into_iter().enumerate() {
            canvas.apply(
                crate::canvas::Origin::Ai,
                CanvasOp::Insert {
                    sheet: crate::protocol::INITIAL_SHEET_ID.into(),
                    stroke,
                    index,
                },
            );
        }
        canvas
    }

    /// Read a PNG back and turn it into (width, height, RGB pixels)
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgb);
        (info.width, info.height, pixels)
    }

    #[test]
    fn the_default_file_name_follows_the_format() {
        assert_eq!(
            default_file_name(Some("2026-11-19-1530"), Some(2), Format::Png),
            "draw-mcp-2026-11-19-1530-sheet-2.png"
        );
        assert_eq!(
            default_file_name(Some("2026-11-19-1530"), None, Format::Pdf),
            "draw-mcp-2026-11-19-1530.pdf"
        );
        assert_eq!(
            default_file_name(None, Some(1), Format::Png),
            "draw-mcp-sheet-1.png"
        );
    }

    #[test]
    fn the_extension_is_added_only_when_it_is_missing_or_different() {
        let png = Format::Png;
        assert_eq!(with_extension(Path::new("a"), png), PathBuf::from("a.png"));
        assert_eq!(
            with_extension(Path::new("a.PNG"), png),
            PathBuf::from("a.PNG")
        );
        assert_eq!(
            with_extension(Path::new("a.pdf"), png),
            PathBuf::from("a.pdf.png")
        );
        assert_eq!(
            with_extension(Path::new("a.pdf"), Format::Pdf),
            PathBuf::from("a.pdf")
        );
    }

    #[test]
    fn clamping_keeps_the_area_inside_the_page() {
        let page = PageSize {
            width: 100.0,
            height: 50.0,
        };
        // One that fits inside the sheet is left as is
        assert_eq!(
            clamp_to_page(Rect::from_edges(10.0, 10.0, 40.0, 30.0), page),
            Some(Rect::from_edges(10.0, 10.0, 40.0, 30.0))
        );
        // The part that sticks out is cut
        assert_eq!(
            clamp_to_page(Rect::from_edges(80.0, 40.0, 200.0, 120.0), page),
            Some(Rect::from_edges(80.0, 40.0, 100.0, 50.0))
        );
        // None if they do not overlap
        assert_eq!(
            clamp_to_page(Rect::from_edges(100.0, 0.0, 200.0, 50.0), page),
            None
        );
        assert_eq!(
            clamp_to_page(Rect::from_edges(-100.0, -100.0, -10.0, -10.0), page),
            None
        );
    }

    #[test]
    fn the_current_sheet_png_is_the_area_twice_as_large() {
        let canvas = canvas_with_strokes(vec![stroke("s", 10.0, 10.0)]);
        let area = Rect::from_edges(0.0, 0.0, 40.0, 20.0);
        let png = sheet_png_by_id(&canvas, INITIAL_SHEET_ID, area).unwrap();

        let (width, height, pixels) = decode(&png);
        assert_eq!((width, height), (80, 40), "範囲 × 2");
        let at = |x: usize, y: usize| &pixels[(y * width as usize + x) * 3..][..3];
        assert_eq!(at(20, 20), [0, 0, 0], "筆跡は 2 倍の位置に描かれる");
    }

    #[test]
    fn a_stroke_outside_the_area_is_not_in_the_png() {
        let canvas = canvas_with_strokes(vec![stroke("s", 100.0, 100.0)]);
        let png = sheet_png_by_id(
            &canvas,
            INITIAL_SHEET_ID,
            Rect::from_edges(0.0, 0.0, 40.0, 20.0),
        )
        .unwrap();

        let (_, _, pixels) = decode(&png);
        assert!(
            pixels.chunks(3).all(|p| p == [255, 255, 255]),
            "範囲の外の筆跡は写らない"
        );
    }

    /// The current sheet's PDF makes the area one page (the size is area × 0.75pt)
    #[test]
    fn current_sheet_pdf_is_the_area_as_one_page() {
        let canvas = canvas_with_strokes(vec![stroke("s", 10.0, 10.0)]);
        let page = PageSize {
            width: 100.0,
            height: 50.0,
        };
        let area = Rect::from_edges(20.0, 10.0, 60.0, 40.0);
        let pdf = pdf(&canvas, Target::CurrentSheet, INITIAL_SHEET_ID, area, page).unwrap();

        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains("/MediaBox [0 0 30 22.5]"),
            "ページの大きさは area × 0.75pt(40px = 30pt、30px = 22.5pt)"
        );
        assert!(text.contains("/Count 1"), "1 ページ");
    }

    /// 2x when the longer side is at most 4096 pixels; only when it exceeds that does the longer side become 4096 pixels
    #[test]
    fn the_scale_keeps_the_long_side_at_most_4096_pixels() {
        // An area about the size of an iPad stays at 2x (2360×1640 pixels)
        let area = Rect::from_edges(0.0, 0.0, 1180.0, 820.0);
        assert_eq!(export_scale(area), EXPORT_SCALE);
        let pixels = render::render_area(&[], None, area, export_scale(area));
        assert_eq!((pixels.width, pixels.height), (2360, 1640));

        // A 7608×4320 area uses the scale that makes the longer side 4096 pixels
        let area = Rect::from_edges(0.0, 0.0, 7608.0, 4320.0);
        let scale = export_scale(area);
        assert!(scale < EXPORT_SCALE);
        let pixels = render::render_area(&[], None, area, scale);
        assert_eq!(pixels.width, 4096, "長い辺が 4096 画素");
        assert!(pixels.height < 4096);
    }

    /// The PDF content (Contents): cut by the area, map the canvas coordinates to the page, and write the strokes as lines
    #[test]
    fn the_pdf_content_cuts_the_area_and_draws_the_strokes_as_lines() {
        let one_point = stroke("p", 30.0, 20.0);
        let red = Stroke {
            id: "s".into(),
            author: Author::Human,
            color: "#ff3b30".into(),
            points: vec![
                StrokePoint {
                    x: 10.0,
                    y: 10.0,
                    pressure: 1.0,
                },
                StrokePoint {
                    x: 50.0,
                    y: 30.0,
                    pressure: 1.0,
                },
            ],
        };
        let canvas = canvas_with_strokes(vec![one_point, red]);
        let page = PageSize {
            width: 100.0,
            height: 50.0,
        };
        let area = Rect::from_edges(20.0, 10.0, 60.0, 40.0);
        let pdf = pdf(&canvas, Target::CurrentSheet, INITIAL_SHEET_ID, area, page).unwrap();
        let text = String::from_utf8_lossy(&pdf);

        // 1. Cut everything outside the page (40px × 30px = 30pt × 22.5pt)
        assert!(text.contains("0 0 30 22.5 re W n\n"), "範囲の切り抜き");
        // 2. Map the canvas coordinates to the page (tx = -20 × 0.75, ty = (10 + 30) × 0.75)
        assert!(text.contains("0.75 0 0 -0.75 -15 30 cm\n"), "座標の写像");
        // 4. Color and line per stroke (the width is the average of the radii 3 × 2 = 6)
        assert!(
            text.contains("0.000 0.000 0.000 RG 1 J 1 j\n6 w 30 20 m 30 20 l S\n"),
            "黒の筆跡。点が 1 つの筆跡は長さ 0 の線"
        );
        assert!(
            text.contains("1.000 0.231 0.188 RG 1 J 1 j\n6 w 10 10 m 50 30 l S\n"),
            "赤の筆跡"
        );
    }

    /// An imported image is pasted as the original image (RGB and alpha) with /Im0 in the content
    #[test]
    fn the_pdf_pastes_the_placed_image_with_its_alpha() {
        let canvas = canvas_with_strokes(Vec::new());
        // A 1-pixel PNG of semi-transparent red
        let mut png_bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut png_bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 128]).unwrap();
        writer.finish().unwrap();
        canvas.put_image_data("img", png_bytes);
        canvas.apply(
            crate::canvas::Origin::Ai,
            CanvasOp::SetImage {
                sheet: crate::protocol::INITIAL_SHEET_ID.into(),
                image: ImagePlacement {
                    id: "img".into(),
                    x: 5.0,
                    y: 7.0,
                    width: 10.0,
                    height: 4.0,
                },
            },
        );
        let page = PageSize {
            width: 100.0,
            height: 50.0,
        };
        let pdf = pdf(
            &canvas,
            Target::CurrentSheet,
            INITIAL_SHEET_ID,
            Rect::from(page),
            page,
        )
        .unwrap();
        let text = String::from_utf8_lossy(&pdf);

        // The content has the placement (10 × 4, y bottom edge 11)
        assert!(
            text.contains("q 10 0 0 -4 5 11 cm /Im0 Do Q\n"),
            "中身が配置で /Im0 を貼る"
        );
        // The image is 1×1 RGB, and since alpha (128) is there, there is also an SMask
        assert!(text.contains("/Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB"));
        assert!(text.contains("/DeviceGray"), "α があれば SMask(グレー)");
        assert!(text.contains("/SMask"));
    }

    /// A PDF of a large area (7608×4320) does not use drawn pixels, so the file is small
    #[test]
    fn a_large_area_pdf_does_not_keep_rendered_pixels() {
        let canvas = canvas_with_strokes(vec![stroke("s", 3804.0, 2160.0)]);
        let page = PageSize {
            width: 7608.0,
            height: 4320.0,
        };
        let pdf = pdf(
            &canvas,
            Target::AllSheets,
            INITIAL_SHEET_ID,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            page,
        )
        .unwrap();
        assert!(
            pdf.len() < 1024 * 1024,
            "1MB より小さい(数 MB 以下)のは、ベクターなので画素の領域を確保していないため"
        );
    }

    #[test]
    fn all_sheets_pdf_has_one_page_per_sheet() {
        let canvas = canvas_with_strokes(vec![stroke("s", 10.0, 10.0)]);
        canvas.apply(
            crate::canvas::Origin::Ai,
            CanvasOp::AddSheet {
                id: "b".into(),
                index: 1,
            },
        );
        canvas.apply(
            crate::canvas::Origin::Ai,
            CanvasOp::AddSheet {
                id: "c".into(),
                index: 2,
            },
        );
        let page = PageSize {
            width: 100.0,
            height: 50.0,
        };
        let pdf = pdf(
            &canvas,
            Target::AllSheets,
            INITIAL_SHEET_ID,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            page,
        )
        .unwrap();

        // The Pages object's /Count is the number of sheets
        let text = String::from_utf8_lossy(&pdf);
        let count = text.find("/Count ").unwrap() + "/Count ".len();
        let digits: String = text[count..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        assert_eq!(digits, "3", "全部のシートならシート 1 枚が PDF の 1 ページ");
    }

    /// Even when the current sheet is another one, sheet_png_by_id draws the sheet given by ID
    #[test]
    fn sheet_png_by_id_renders_the_sheet_named_by_id_not_the_current_sheet() {
        // Put a stroke only on the first sheet, and make the second sheet current
        let canvas = canvas_with_strokes(vec![stroke("s", 10.0, 10.0)]);
        canvas.apply(
            crate::canvas::Origin::Ai,
            CanvasOp::AddSheet {
                id: "b".into(),
                index: 1,
            },
        );
        canvas.apply(
            crate::canvas::Origin::Ai,
            CanvasOp::SelectSheet { id: "b".into() },
        );
        let area = Rect::from_edges(0.0, 0.0, 40.0, 20.0);
        let png = sheet_png_by_id(&canvas, INITIAL_SHEET_ID, area).unwrap();

        let (width, height, pixels) = decode(&png);
        assert_eq!((width, height), (80, 40), "範囲 × 2");
        let at = |x: usize, y: usize| &pixels[(y * width as usize + x) * 3..][..3];
        assert_eq!(
            at(20, 20),
            [0, 0, 0],
            "今のシート(b)ではなく 1 枚目の筆跡が写る"
        );
    }

    /// Passing the ID of a missing sheet is an error for both PNG and PDF (CurrentSheet)
    #[test]
    fn a_missing_sheet_id_is_an_error_for_png_and_current_sheet_pdf() {
        let canvas = canvas_with_strokes(Vec::new());
        let page = PageSize {
            width: 100.0,
            height: 50.0,
        };
        let area = Rect::from(page);
        assert_eq!(
            sheet_png_by_id(&canvas, "gone", area).err(),
            Some("The sheet was deleted.".to_string())
        );
        assert_eq!(
            pdf(&canvas, Target::CurrentSheet, "gone", area, page).err(),
            Some("The sheet was deleted.".to_string())
        );
    }
}
