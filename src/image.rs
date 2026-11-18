use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::Path;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Area {
    pub fn between(a: Point, b: Point) -> Self {
        Self {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            w: (a.x - b.x).abs(),
            h: (a.y - b.y).abs(),
        }
    }

    pub fn intersect(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Self {
            x,
            y,
            w: ((self.x + self.w).min(other.x + other.w) - x).max(0),
            h: ((self.y + self.h).min(other.y + other.h) - y).max(0),
        }
    }

    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.x + self.w && p.y < self.y + self.h
    }
}

/// Top-down BGRA, with opaque alpha. This is also the native Win32 DIB layout.
#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .filter(|n| *n > 0 && *n <= 512 * 1024 * 1024)
            .ok_or("Image is empty or exceeds the 512 MiB safety limit")?;
        Ok(Self {
            width,
            height,
            pixels: vec![255; size],
        })
    }

    pub fn bounds(&self) -> Area {
        Area {
            x: 0,
            y: 0,
            w: self.width as i32,
            h: self.height as i32,
        }
    }

    pub fn crop(&self, area: Area) -> Result<Self> {
        let a = area.intersect(self.bounds());
        let mut image = Self::new(a.w as u32, a.h as u32)?;
        for y in 0..a.h as usize {
            let src = ((a.y as usize + y) * self.width as usize + a.x as usize) * 4;
            let dst = y * a.w as usize * 4;
            image.pixels[dst..dst + a.w as usize * 4]
                .copy_from_slice(&self.pixels[src..src + a.w as usize * 4]);
        }
        Ok(image)
    }

    pub fn thumbnail(&self, max_w: u32, max_h: u32) -> Self {
        let ratio = (max_w as f64 / self.width as f64)
            .min(max_h as f64 / self.height as f64)
            .min(1.0);
        let w = (self.width as f64 * ratio).round().max(1.0) as u32;
        let h = (self.height as f64 * ratio).round().max(1.0) as u32;
        let mut out = Self::new(w, h).expect("bounded thumbnail");
        for y in 0..h {
            for x in 0..w {
                let src = ((y * self.height / h) * self.width + x * self.width / w) as usize * 4;
                let dst = (y * w + x) as usize * 4;
                out.pixels[dst..dst + 4].copy_from_slice(&self.pixels[src..src + 4]);
            }
        }
        out
    }

    /// Never overwrite an existing screenshot. A partial file is removed on error.
    pub fn save_png(&self, path: &Path) -> Result<()> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            let mut encoder = png::Encoder::new(BufWriter::new(file), self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
            let mut rgba = self.pixels.clone();
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
            writer.finish().map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(path);
        }
        result
    }

    pub fn load_png(path: &Path) -> Result<Self> {
        let mut decoder =
            png::Decoder::new(BufReader::new(File::open(path).map_err(|e| e.to_string())?));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let info = reader.info();
        let mut out = Self::new(info.width, info.height)?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("Invalid PNG size")?];
        let frame = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let channels = frame.color_type.samples();
        for (dst, src) in out
            .pixels
            .chunks_exact_mut(4)
            .zip(buf[..frame.buffer_size()].chunks_exact(channels))
        {
            match channels {
                4 => dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]),
                3 => dst.copy_from_slice(&[src[2], src[1], src[0], 255]),
                2 => dst.copy_from_slice(&[src[0], src[0], src[0], src[1]]),
                _ => dst.copy_from_slice(&[src[0], src[0], src[0], 255]),
            }
        }
        Ok(out)
    }

    fn dot(&mut self, p: Point, color: [u8; 4], radius: i32) {
        for y in (p.y - radius).max(0)..=(p.y + radius).min(self.height as i32 - 1) {
            for x in (p.x - radius).max(0)..=(p.x + radius).min(self.width as i32 - 1) {
                if (x - p.x).pow(2) + (y - p.y).pow(2) <= radius * radius {
                    let i = (y as usize * self.width as usize + x as usize) * 4;
                    self.pixels[i..i + 4].copy_from_slice(&color);
                }
            }
        }
    }

    fn line(&mut self, a: Point, b: Point, color: [u8; 4], radius: i32) {
        let (mut x, mut y) = (a.x, a.y);
        let (dx, dy) = ((b.x - a.x).abs(), -(b.y - a.y).abs());
        let (sx, sy) = (
            if a.x < b.x { 1 } else { -1 },
            if a.y < b.y { 1 } else { -1 },
        );
        let mut err = dx + dy;
        loop {
            self.dot(Point { x, y }, color, radius);
            if x == b.x && y == b.y {
                break;
            }
            let e = 2 * err;
            if e >= dy {
                err += dy;
                x += sx;
            }
            if e <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    pub fn annotate(&mut self, mark: &Mark) {
        let Some(&a) = mark.points.first() else {
            return;
        };
        let b = *mark.points.last().unwrap_or(&a);
        let color = mark.color;
        let r = mark.radius;
        match mark.tool {
            Tool::Pen => {
                self.dot(a, color, r);
                for pair in mark.points.windows(2) {
                    self.line(pair[0], pair[1], color, r);
                }
            }
            Tool::Arrow => {
                self.line(a, b, color, r);
                let theta = ((a.y - b.y) as f64).atan2((a.x - b.x) as f64);
                let len = ((a.x - b.x) as f64)
                    .hypot((a.y - b.y) as f64)
                    .mul_add(0.25, 0.0)
                    .clamp(6.0, 24.0 + r as f64 * 3.0);
                for angle in [theta - 0.5, theta + 0.5] {
                    self.line(
                        b,
                        Point {
                            x: b.x + (angle.cos() * len) as i32,
                            y: b.y + (angle.sin() * len) as i32,
                        },
                        color,
                        r,
                    );
                }
            }
            Tool::Rectangle => {
                let c = Point { x: b.x, y: a.y };
                let d = Point { x: a.x, y: b.y };
                for (start, end) in [(a, c), (c, b), (b, d), (d, a)] {
                    self.line(start, end, color, r);
                }
            }
            Tool::Redact => {
                let area = Area::between(a, b).intersect(self.bounds());
                // Solid, opaque redaction. No hidden original pixels in the exported PNG.
                for y in area.y..area.y + area.h {
                    for x in area.x..area.x + area.w {
                        let i = (y as usize * self.width as usize + x as usize) * 4;
                        self.pixels[i..i + 4].copy_from_slice(&[20, 20, 20, 255]);
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Arrow,
    Rectangle,
    Pen,
    Redact,
}

#[derive(Clone)]
pub struct Mark {
    pub tool: Tool,
    pub points: Vec<Point>,
    pub color: [u8; 4],
    pub radius: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_selection_and_negative_monitor_coordinates() {
        let area = Area::between(Point { x: 100, y: 80 }, Point { x: -200, y: -50 });
        assert_eq!(
            area,
            Area {
                x: -200,
                y: -50,
                w: 300,
                h: 130
            }
        );
        assert_eq!(
            area.intersect(Area {
                x: 0,
                y: 0,
                w: 50,
                h: 50
            }),
            Area {
                x: 0,
                y: 0,
                w: 50,
                h: 50
            }
        );
    }

    #[test]
    fn crop_preserves_channels_and_top_down_orientation() {
        let mut image = Image::new(4, 3).unwrap();
        for y in 0..3 {
            for x in 0..4 {
                let i = (y * 4 + x) * 4;
                image.pixels[i..i + 4].copy_from_slice(&[x as u8, y as u8, 200, 255]);
            }
        }
        let crop = image
            .crop(Area {
                x: 2,
                y: 1,
                w: 20,
                h: 20,
            })
            .unwrap();
        assert_eq!((crop.width, crop.height), (2, 2));
        assert_eq!(&crop.pixels[..4], &[2, 1, 200, 255]);
        assert_eq!(&crop.pixels[12..], &[3, 2, 200, 255]);
        assert!(image
            .crop(Area {
                x: 8,
                y: 8,
                w: 1,
                h: 1
            })
            .is_err());
    }

    #[test]
    fn redaction_removes_every_original_pixel_within_bounds() {
        let mut image = Image::new(20, 20).unwrap();
        image.annotate(&Mark {
            tool: Tool::Redact,
            points: vec![Point { x: 12, y: 13 }, Point { x: 3, y: 4 }],
            color: [0; 4],
            radius: 2,
        });
        for y in 4..13 {
            for x in 3..12 {
                assert_eq!(
                    &image.pixels[(y * 20 + x) * 4..(y * 20 + x) * 4 + 4],
                    &[20, 20, 20, 255]
                );
            }
        }
        assert_eq!(&image.pixels[..4], &[255; 4]);
    }

    #[test]
    fn png_roundtrip_is_lossless_and_existing_files_are_preserved() {
        let path =
            std::env::temp_dir().join(format!("feathershot-test-{}.png", std::process::id()));
        let mut image = Image::new(3, 2).unwrap();
        image.pixels[..8].copy_from_slice(&[40, 80, 180, 255, 200, 30, 10, 255]);
        image.save_png(&path).unwrap();
        assert!(image.save_png(&path).is_err());
        let loaded = Image::load_png(&path).unwrap();
        assert_eq!(loaded.pixels, image.pixels);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reject_invalid_allocation_and_draw_clipped_lines() {
        assert!(Image::new(0, 20).is_err());
        assert!(Image::new(u32::MAX, u32::MAX).is_err());
        let mut image = Image::new(10, 10).unwrap();
        let mark = Mark {
            tool: Tool::Pen,
            points: vec![Point { x: -2, y: -2 }, Point { x: 12, y: 12 }],
            color: [1, 2, 3, 255],
            radius: 2,
        };
        image.annotate(&mark);
        assert_eq!(&image.pixels[..4], &[1, 2, 3, 255]);
        assert_eq!(&image.pixels[396..], &[1, 2, 3, 255]);
    }
}
