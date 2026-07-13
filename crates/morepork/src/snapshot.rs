//! Family-agnostic snapshot payloads: bulk memory regions and indexed
//! screen frames. Family-specific typed payloads (the `gb.*` kinds) live
//! with their family (`system::gb::snapshot`).

/// Memory snapshot payload.
///
/// Format: [num_regions: u8] then for each region:
///   [start_addr: u16 LE] [length: u16 LE] [data: u8 * length]
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    pub start: u16,
    pub data: Vec<u8>,
}

pub fn parse_memory_snapshot(payload: &[u8]) -> Option<Vec<MemoryRegion>> {
    if payload.is_empty() { return None; }
    let num_regions = payload[0] as usize;
    let mut pos = 1;
    let mut regions = Vec::with_capacity(num_regions);
    for _ in 0..num_regions {
        if pos + 4 > payload.len() { return None; }
        let start = u16::from_le_bytes([payload[pos], payload[pos + 1]]);
        let len = u16::from_le_bytes([payload[pos + 2], payload[pos + 3]]) as usize;
        pos += 4;
        if pos + len > payload.len() { return None; }
        regions.push(MemoryRegion {
            start,
            data: payload[pos..pos + len].to_vec(),
        });
        pos += len;
    }
    Some(regions)
}

pub fn build_memory_payload(regions: &[MemoryRegion]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(regions.len() as u8);
    for region in regions {
        out.extend_from_slice(&region.start.to_le_bytes());
        out.extend_from_slice(&(region.data.len() as u16).to_le_bytes());
        out.extend_from_slice(&region.data);
    }
    out
}

/// Indexed screen-frame snapshot payload — the family-agnostic `frame`
/// kind for systems whose display is palette-indexed
/// (`PixFormat::Indexed8`). Mirrors missingno's `IndexedFrame`: per-frame
/// dimensions (VCS frame height is emergent), the palette as it stood at
/// frame end (SMS CRAM is mutable), and the display pixel aspect.
///
/// GB traces do not use this: their `frame` payloads remain raw pixel
/// bytes in the header's `pix_format` (`shade2`/`rgb555`) at 160×144, as
/// they always have been.
///
/// Format (all little-endian):
///   [width: u16] [height: u16] [pixel_aspect: f32]
///   [palette_len: u16] [palette: 3 bytes RGB × palette_len]
///   [pixels: u8 × width×height]
#[derive(Debug, Clone, PartialEq)]
pub struct IndexedFrame {
    pub width: u16,
    pub height: u16,
    pub pixel_aspect: f32,
    pub palette: Vec<[u8; 3]>,
    pub pixels: Vec<u8>,
}

impl IndexedFrame {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(10 + self.palette.len() * 3 + self.pixels.len());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.pixel_aspect.to_le_bytes());
        out.extend_from_slice(&(self.palette.len() as u16).to_le_bytes());
        for rgb in &self.palette {
            out.extend_from_slice(rgb);
        }
        out.extend_from_slice(&self.pixels);
        out
    }

    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 10 { return None; }
        let width = u16::from_le_bytes([data[0], data[1]]);
        let height = u16::from_le_bytes([data[2], data[3]]);
        let pixel_aspect = f32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let palette_len = u16::from_le_bytes([data[8], data[9]]) as usize;
        let mut pos = 10;
        if data.len() < pos + palette_len * 3 { return None; }
        let palette: Vec<[u8; 3]> = (0..palette_len)
            .map(|i| [data[pos + i * 3], data[pos + i * 3 + 1], data[pos + i * 3 + 2]])
            .collect();
        pos += palette_len * 3;
        let expected = width as usize * height as usize;
        if data.len() < pos + expected { return None; }
        Some(IndexedFrame {
            width,
            height,
            pixel_aspect,
            palette,
            pixels: data[pos..pos + expected].to_vec(),
        })
    }

    /// Resolve to RGBA8 for display.
    pub fn to_rgba(&self) -> Vec<u8> {
        self.pixels
            .iter()
            .flat_map(|&i| {
                let [r, g, b] = self.palette.get(i as usize).copied().unwrap_or([0, 0, 0]);
                [r, g, b, 255]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_frame_roundtrip() {
        let frame = IndexedFrame {
            width: 160,
            height: 192,
            pixel_aspect: 1.6,
            palette: vec![[0, 0, 0], [255, 0, 0], [0, 255, 0]],
            pixels: (0..160u32 * 192).map(|i| (i % 3) as u8).collect(),
        };
        let bytes = frame.to_bytes();
        let back = IndexedFrame::from_bytes(&bytes).unwrap();
        assert_eq!(frame, back);
        assert_eq!(back.to_rgba().len(), 160 * 192 * 4);
    }

    #[test]
    fn indexed_frame_rejects_truncated() {
        let frame = IndexedFrame {
            width: 4,
            height: 4,
            pixel_aspect: 1.0,
            palette: vec![[1, 2, 3]],
            pixels: vec![0; 16],
        };
        let bytes = frame.to_bytes();
        assert!(IndexedFrame::from_bytes(&bytes[..bytes.len() - 1]).is_none());
        assert!(IndexedFrame::from_bytes(&[]).is_none());
    }
}
