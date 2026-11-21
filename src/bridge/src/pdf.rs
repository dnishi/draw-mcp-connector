//! Builds a PDF with page content (stroke lines, etc.) and images.
//!
//! This file writes the objects in number order and builds the cross-reference table. Building the
//! page content (Contents) is done in export.rs. The object order and the shape of the
//! cross-reference table follow ISO 32000-1:2008. Image data is a sequence of rows, each prefixed
//! with a PNG filter type, compressed with zlib (the same shape as concatenated PNG IDAT data),
//! so it can be read directly with FlateDecode and PNG prediction (Predictor 15).

/// An image to paste into the PDF (built from RGBA colors without alpha premultiplication)
pub struct PdfImage {
    pub width: u32,
    pub height: u32,
    /// RGB data with a filter type prefixed to each row, compressed with zlib
    pub rgb: Vec<u8>,
    /// Alpha (one channel of the same shape). None if all are 255
    pub alpha: Option<Vec<u8>>,
}

/// One page of the PDF
pub struct PdfPage {
    pub width_pt: f64,
    pub height_pt: f64,
    /// The page content (the Contents stream)
    pub content: Vec<u8>,
    /// The image the content uses with `/Im0`
    pub image: Option<PdfImage>,
}

/// The PDF bytes of the pages in order
pub fn write_pdf(pages: &[PdfPage]) -> Vec<u8> {
    let mut pdf = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    // The second line indicates the file contains binary data
    pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    // Object numbers: assigned from 1 in the order they are written (1 = Catalog, 2 = Pages).
    // Content, image, and SMask take the next number when written. Whether a page has an image
    // or an SMask differs per page, so count the objects per page first to fix the numbers
    // in Pages' /Kids
    let per_page: Vec<u32> = pages
        .iter()
        .map(|page| {
            2 + u32::from(page.image.is_some())
                + u32::from(
                    page.image
                        .as_ref()
                        .is_some_and(|image| image.alpha.is_some()),
                )
        })
        .collect();
    dict_object(
        &mut pdf,
        &mut offsets,
        1,
        "<< /Type /Catalog /Pages 2 0 R >>",
    );
    let mut page_numbers = Vec::new();
    let mut next = 3;
    for size in &per_page {
        page_numbers.push(next);
        next += size;
    }
    let kids = page_numbers
        .iter()
        .map(|number| format!("{number} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    dict_object(
        &mut pdf,
        &mut offsets,
        2,
        &format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len()),
    );
    for (page, number) in pages.iter().zip(&page_numbers) {
        // Page and Contents are always present. Without an image /Resources is empty,
        // and there is no SMask when the image has no alpha
        let (image_number, smask_number) = match &page.image {
            Some(image) => (Some(number + 2), image.alpha.as_ref().map(|_| number + 3)),
            None => (None, None),
        };
        write_page(
            &mut pdf,
            &mut offsets,
            *number,
            page,
            number + 1,
            image_number,
        );
        write_contents(&mut pdf, &mut offsets, number + 1, &page.content);
        if let (Some(image), Some(image_number)) = (&page.image, image_number) {
            write_image_object(
                &mut pdf,
                &mut offsets,
                image_number,
                &image_dict(image.width, image.height, "/DeviceRGB", 3, smask_number),
                &image.rgb,
            );
            if let Some(smask_number) = smask_number {
                write_image_object(
                    &mut pdf,
                    &mut offsets,
                    smask_number,
                    &image_dict(image.width, image.height, "/DeviceGray", 1, None),
                    image.alpha.as_ref().expect("α があれば SMask も作る"),
                );
            }
        }
    }

    // Cross-reference table: record the start position of each object (each row exactly 20 bytes)
    let xref_position = pdf.len();
    let size = offsets.len() + 1; // The free object (number 0) plus each object
    pdf.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_position}\n%%EOF\n")
            .as_bytes(),
    );
    pdf
}

/// Page: size in pt, content is the Contents stream, image is an XObject
fn write_page(
    pdf: &mut Vec<u8>,
    offsets: &mut Vec<usize>,
    number: u32,
    page: &PdfPage,
    contents: u32,
    image_number: Option<u32>,
) {
    let resources = match image_number {
        Some(number) => format!("<< /XObject << /Im0 {number} 0 R >> >>"),
        None => "<< >>".to_string(),
    };
    let (width, height) = (pt(page.width_pt), pt(page.height_pt));
    dict_object(
        pdf,
        offsets,
        number,
        &format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources {resources} /Contents {contents} 0 R >>"
        ),
    );
}

/// Contents: uncompressed. /Length is the number of bytes
fn write_contents(pdf: &mut Vec<u8>, offsets: &mut Vec<usize>, number: u32, content: &[u8]) {
    start_object(pdf, offsets, number);
    pdf.extend_from_slice(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
    pdf.extend_from_slice(content);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
}

/// The image dictionary. The stream data and /Length are added by `write_image_object`.
/// /SMask is attached only when there is an smask
fn image_dict(
    width: u32,
    height: u32,
    color_space: &str,
    colors: u8,
    smask: Option<u32>,
) -> String {
    let smask_ref = smask
        .map(|number| format!(" /SMask {number} 0 R"))
        .unwrap_or_default();
    format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {color_space} /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors {colors} /BitsPerComponent 8 /Columns {width} >>{smask_ref}"
    )
}

/// Image: after the dictionary, write zlib data (a sequence of rows, each prefixed with a PNG
/// filter type)
fn write_image_object(
    pdf: &mut Vec<u8>,
    offsets: &mut Vec<usize>,
    number: u32,
    dict: &str,
    data: &[u8],
) {
    start_object(pdf, offsets, number);
    pdf.extend_from_slice(format!("{dict} /Length {} >>\nstream\n", data.len()).as_bytes());
    pdf.extend_from_slice(data);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
}

/// Record the start position of the object and write `N 0 obj`
fn start_object(pdf: &mut Vec<u8>, offsets: &mut Vec<usize>, number: u32) {
    offsets.push(pdf.len());
    pdf.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
}

/// Write an object with only a dictionary
fn dict_object(pdf: &mut Vec<u8>, offsets: &mut Vec<usize>, number: u32, dict: &str) {
    start_object(pdf, offsets, number);
    pdf.extend_from_slice(format!("{dict}\nendobj\n").as_bytes());
}

/// Format a number (pt) to at most 3 decimal places, dropping trailing zeros and the point (600 → `600`, 450.75 → `450.75`)
pub(crate) fn pt(value: f64) -> String {
    let formatted = format!("{value:.3}");
    let text = formatted.trim_end_matches('0');
    text.strip_suffix('.').unwrap_or(text).to_string()
}

/// Build a PdfImage from RGBA colors without alpha premultiplication.
/// In memory, build an RGB PNG (color type 2) and an alpha PNG (grayscale, color type 0;
/// absent when all alpha is 255), and use the concatenated IDAT data of each
pub fn image_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<PdfImage, String> {
    let pixels = width as usize * height as usize;
    if rgba.len() != pixels * 4 {
        return Err("RGBA の長さが幅と高さに合わない".to_string());
    }
    let (mut rgb, mut alpha) = (Vec::with_capacity(pixels * 3), Vec::with_capacity(pixels));
    // The length is checked above, so the remainder (the second element) is empty
    let (pixels, _) = rgba.as_chunks::<4>();
    for &[r, g, b, a] in pixels {
        rgb.extend_from_slice(&[r, g, b]);
        alpha.push(a);
    }
    let rgb = png_idat(width, height, png::ColorType::Rgb, &rgb)?;
    // No SMask is needed when all alpha is 255
    let alpha = if alpha.iter().any(|&value| value != 255) {
        Some(png_idat(width, height, png::ColorType::Grayscale, &alpha)?)
    } else {
        None
    };
    Ok(PdfImage {
        width,
        height,
        rgb,
        alpha,
    })
}

/// Build a PNG of the given color type in memory from 8bit pixels and return
/// the concatenated IDAT data
fn png_idat(
    width: u32,
    height: u32,
    color_type: png::ColorType,
    pixels: &[u8],
) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(color_type);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(pixels)
        .map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    let (_, _, idat) = png_image(&png)?;
    Ok(idat)
}

/// Extract the width, height, and concatenated IDAT data from a PNG.
/// Errors unless it is 8bit, non-interlaced (0) RGB (color type 2) or grayscale (0). CRC is not verified
fn png_image(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    // Chunk layout: 4 bytes length, 4 bytes type, data, 4 bytes CRC
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    let Some(rest) = png.strip_prefix(SIGNATURE) else {
        return Err("not a PNG".to_string());
    };
    let mut rest = rest;
    let mut size: Option<(u32, u32)> = None;
    let mut idat: Vec<u8> = Vec::new();
    while !rest.is_empty() {
        let length =
            u32::from_be_bytes(rest.get(..4).ok_or("incomplete chunk")?.try_into().unwrap())
                as usize;
        let Some(chunk) = rest.get(4..8 + length + 4) else {
            return Err("incomplete chunk".to_string());
        };
        let (kind, payload) = chunk.split_at(4);
        let data = &payload[..length];
        match kind {
            b"IHDR" => size = Some(ihdr(data)?),
            b"IDAT" => idat.extend_from_slice(data),
            _ => {}
        }
        rest = &rest[8 + length + 4..];
    }
    size.ok_or_else(|| "missing IHDR".to_string())
        .map(|(width, height)| (width, height, idat))
}

/// Read IHDR and verify it is 8bit, non-interlaced (0) RGB (color type 2) or grayscale (0)
fn ihdr(data: &[u8]) -> Result<(u32, u32), String> {
    // width 4, height 4, bit depth 1, color type 1, compression 1, filter 1, interlace 1
    if data.len() != 13 {
        return Err("invalid IHDR".to_string());
    }
    let width = u32::from_be_bytes(data[0..4].try_into().unwrap());
    let height = u32::from_be_bytes(data[4..8].try_into().unwrap());
    let (depth, color_type, interlace) = (data[8], data[9], data[12]);
    if depth != 8 || (color_type != 0 && color_type != 2) || interlace != 0 {
        return Err("PNG must be 8bit without interlace".to_string());
    }
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page with empty content and no image
    fn page(width_pt: f64, height_pt: f64) -> PdfPage {
        PdfPage {
            width_pt,
            height_pt,
            content: Vec::new(),
            image: None,
        }
    }

    /// A page whose content pastes a 2×2 image (alpha 128 and 255, so it has an SMask) with /Im0
    fn page_with_image() -> PdfPage {
        let rgba = [255, 0, 0, 128, 0, 0, 255, 128, 1, 2, 3, 255, 4, 5, 6, 255];
        let mut page = page(150.0, 100.0);
        page.image = Some(image_from_rgba(2, 2, &rgba).unwrap());
        page.content = b"q 2 0 0 -2 0 2 cm /Im0 Do Q\n".to_vec();
        page
    }

    /// Read each object's offset (in number order) from the cross-reference table.
    /// Read from the position startxref points to (so it works even if image data contains "xref")
    fn xref_offsets(pdf: &[u8]) -> Vec<usize> {
        // The trailer's startxref is the last one in the file
        let marker = pdf
            .windows(b"startxref".len())
            .rposition(|w| w == b"startxref")
            .unwrap();
        let mut pos = marker + b"startxref".len() + 1; // "startxref\n"
        let line_end = pdf[pos..].iter().position(|&b| b == b'\n').unwrap() + pos;
        let xref = std::str::from_utf8(&pdf[pos..line_end])
            .unwrap()
            .parse::<usize>()
            .unwrap();
        pos = xref + 5; // "xref\n"
        let line_end = pdf[pos..].iter().position(|&b| b == b'\n').unwrap() + pos;
        let count = std::str::from_utf8(&pdf[pos + 2..line_end])
            .unwrap()
            .parse::<usize>()
            .unwrap();
        pos = line_end + 1;
        (0..count)
            .map(|_| {
                let offset = std::str::from_utf8(&pdf[pos..pos + 10])
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                pos += 20;
                offset
            })
            .collect()
    }

    /// Read all single lines (object dictionaries) containing `kind`
    fn lines_containing(pdf: &[u8], kind: &str) -> Vec<String> {
        String::from_utf8_lossy(pdf)
            .lines()
            .filter(|line| line.contains(kind))
            .map(str::to_string)
            .collect()
    }

    /// Read the number right after `name` from a dictionary string
    fn dict_number(dict: &str, name: &str) -> u32 {
        let rest = &dict[dict.find(name).unwrap() + name.len()..];
        let start = rest.find(|c: char| c.is_ascii_digit()).unwrap();
        let end = rest[start..].find(|c: char| !c.is_ascii_digit()).unwrap();
        rest[start..start + end].parse().unwrap()
    }

    /// Extract the bytes of the stream (the data part) right after a dictionary line
    fn stream_after_dict(pdf: &[u8], dict: &str) -> Vec<u8> {
        let dict_start = pdf
            .windows(dict.len())
            .position(|w| w == dict.as_bytes())
            .unwrap()
            + dict.len();
        let stream = dict_start
            + pdf[dict_start..]
                .windows(b"stream\n".len())
                .position(|w| w == b"stream\n")
                .unwrap()
            + b"stream\n".len();
        let length = dict_number(dict, "/Length") as usize;
        pdf[stream..stream + length].to_vec()
    }

    #[test]
    fn the_xref_points_to_the_start_of_each_object() {
        // Pages: no image (3), image without alpha (4), with alpha (5)
        let opaque = {
            let mut page = page(300.0, 200.0);
            page.image = Some(
                image_from_rgba(
                    2,
                    2,
                    &[1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255],
                )
                .unwrap(),
            );
            page
        };
        for (pages, per_page) in [
            (vec![page(600.0, 450.0)], vec![2]),
            (vec![page(600.0, 450.0), page(300.0, 200.0)], vec![2, 2]),
            (
                vec![page(600.0, 450.0), page_with_image(), opaque],
                vec![2, 4, 3],
            ),
        ] {
            let pdf = write_pdf(&pages);
            let offsets = xref_offsets(&pdf);
            assert_eq!(
                offsets.len(),
                3 + per_page.iter().sum::<usize>(),
                "Catalog・Pages・ページごとのオブジェクト・空きオブジェクト"
            );
            assert_eq!(offsets[0], 0, "番号 0 は先頭(空き)");
            for (number, offset) in offsets.iter().enumerate().skip(1) {
                assert!(
                    pdf[*offset..].starts_with(format!("{number} 0 obj").as_bytes()),
                    "番号 {number} の位置が `{number} 0 obj` の先頭を指す"
                );
            }
        }
    }

    #[test]
    fn the_pages_object_has_the_count_and_the_kids() {
        let pdf = write_pdf(&[page(600.0, 450.0), page_with_image()]);
        let text = String::from_utf8_lossy(&pdf);
        // A page without an image uses 2 objects, a page with alpha in the image uses 4
        assert!(
            text.contains("<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>"),
            "/Kids はページごとにオブジェクトの数だけ進み、/Count はページ数"
        );
        // Numbers are written with trailing zeros and the point dropped
        assert!(text.contains("/MediaBox [0 0 600 450]"));
        let pdf = write_pdf(&[page(450.75, 200.0)]);
        assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 450.75 200]"));
    }

    #[test]
    fn the_page_references_the_contents_and_the_image() {
        let pdf = write_pdf(&[page_with_image()]);
        let page_dict = lines_containing(&pdf, "/Type /Page ").pop().unwrap();
        // Contents (4), image (5), SMask (6)
        assert!(
            page_dict.contains("/Resources << /XObject << /Im0 5 0 R >> >>"),
            "画像があるページは /Im0 を指す"
        );
        assert!(page_dict.contains("/Contents 4 0 R"));
        assert!(
            String::from_utf8_lossy(&pdf).contains("/Im0 Do"),
            "中身の /Im0 がそのまま入る"
        );

        // A page without an image has an empty /Resources
        let pdf = write_pdf(&[page(100.0, 100.0)]);
        let page_dict = lines_containing(&pdf, "/Type /Page ").pop().unwrap();
        assert!(
            page_dict.contains("/Resources << >>"),
            "画像が無いページは /Resources が空"
        );
    }

    #[test]
    fn the_contents_stream_is_written_uncompressed() {
        let content = b"0 0 600 450 re W n";
        let mut page = page(800.0, 600.0);
        page.content = content.to_vec();
        let pdf = write_pdf(&[page]);
        // /Length is the byte count of the content, and the content goes in as-is
        assert!(
            String::from_utf8_lossy(&pdf)
                .contains("<< /Length 18 >>\nstream\n0 0 600 450 re W n\nendstream\nendobj\n"),
            "Contents は圧縮しない"
        );
    }

    #[test]
    fn the_image_object_matches_the_rgba() {
        let pdf = write_pdf(&[page_with_image()]);
        // Two Image objects: RGB (5) and SMask (6)
        let dicts = lines_containing(&pdf, "/Subtype /Image");
        assert_eq!(dicts.len(), 2, "RGB と SMask の 2 つ");
        let color: Vec<&String> = dicts
            .iter()
            .filter(|dict| dict.contains("/DeviceRGB"))
            .collect();
        let smask: Vec<&String> = dicts
            .iter()
            .filter(|dict| !dict.contains("/DeviceRGB"))
            .collect();
        assert_eq!(color.len(), 1);
        assert_eq!(smask.len(), 1);
        assert_eq!(dict_number(color[0], "/Width"), 2);
        assert_eq!(dict_number(color[0], "/Height"), 2);
        assert_eq!(dict_number(color[0], "/Columns"), 2);
        assert_eq!(dict_number(color[0], "/Colors"), 3);
        assert!(color[0].contains("/Predictor 15"));
        assert_eq!(
            dict_number(color[0], "/SMask"),
            6,
            "α がある画像は /SMask を指す"
        );
        assert!(smask[0].contains("/DeviceGray"));
        assert_eq!(dict_number(smask[0], "/Colors"), 1);
        assert_eq!(dict_number(smask[0], "/Columns"), 2);

        // The stream is zlib (the first 2 bytes `78 xx`), and the length is /Length
        for dict in &[color[0].as_str(), smask[0].as_str()] {
            let data = stream_after_dict(&pdf, dict);
            assert_eq!(data.len(), dict_number(dict, "/Length") as usize);
            assert_eq!(data[0], 0x78, "zlib のヘッダ");
        }
    }

    #[test]
    fn alpha_that_is_all_opaque_has_no_smask() {
        let mut page = page(100.0, 100.0);
        page.image = Some(image_from_rgba(2, 1, &[9, 9, 9, 255, 8, 8, 8, 255]).unwrap());
        assert!(
            page.image.as_ref().unwrap().alpha.is_none(),
            "全部 255 なら None"
        );
        let pdf = write_pdf(&[page]);
        assert!(
            !String::from_utf8_lossy(&pdf).contains("/SMask"),
            "全部 255 なら /SMask が無い"
        );
        // Only Contents (4) and one Image (5), and /Im0 points to it
        let page_dict = lines_containing(&pdf, "/Type /Page ").pop().unwrap();
        assert!(page_dict.contains("/Im0 5 0 R"));
    }

    #[test]
    fn image_from_rgba_checks_the_length() {
        assert!(
            image_from_rgba(2, 1, &[0; 3]).is_err(),
            "RGBA の長さが幅と高さに合わない"
        );
        let image = image_from_rgba(2, 1, &[0; 8]).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
    }
}
