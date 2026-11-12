//! A screen's real size, for measuring in cm and inches. Read from what the
//! monitor tells the computer about itself (its EDID, in /sys/class/drm),
//! which works the same on X11 and Wayland. A screen that doesn't say (some
//! projectors, virtual screens) measures in px only.

use std::sync::OnceLock;

/// What one connected monitor reports: its connector ("DP-1") and its
/// picture's width in mm.
#[derive(Debug, Clone, PartialEq)]
struct Reported {
    connector: String,
    width_mm: u32,
}

/// The picture's width and height in mm, from a raw EDID block: the first
/// detailed timing's image size, else the basic size in cm. None if it
/// doesn't look like an EDID or gives no size.
fn size_mm(edid: &[u8]) -> Option<(u32, u32)> {
    if edid.len() < 128 || edid[..8] != [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00] {
        return None;
    }
    let dtd = &edid[54..72];
    // a detailed timing has a pixel clock; its image size is in mm
    if dtd[0] != 0 || dtd[1] != 0 {
        let w = u32::from(dtd[12]) | (u32::from(dtd[14] >> 4) << 8);
        let h = u32::from(dtd[13]) | (u32::from(dtd[14] & 0x0F) << 8);
        if w > 0 && h > 0 {
            return Some((w, h));
        }
    }
    let (w_cm, h_cm) = (u32::from(edid[21]), u32::from(edid[22]));
    (w_cm > 0 && h_cm > 0).then_some((w_cm * 10, h_cm * 10))
}

fn reported() -> &'static [Reported] {
    static ALL: OnceLock<Vec<Reported>> = OnceLock::new();
    ALL.get_or_init(|| {
        let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return vec![] };
        let mut out = Vec::new();
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // "card0-DP-1" → "DP-1" (the cards themselves have no connector part)
            let Some((_, connector)) = name.split_once('-') else { continue };
            let connected = std::fs::read_to_string(entry.path().join("status")).is_ok_and(|s| s.trim() == "connected");
            let Ok(edid) = std::fs::read(entry.path().join("edid")) else { continue };
            if let (true, Some((width_mm, _))) = (connected, size_mm(&edid)) {
                out.push(Reported { connector: connector.to_string(), width_mm });
            }
        }
        out
    })
}

/// mm per screen pixel on the monitor named `name` (winit's name for it,
/// e.g. "DP-1") that is `width_px` wide, if it reports its size. With one
/// screen reporting a size, that one is used whatever its name.
pub(crate) fn mm_per_px(name: Option<&str>, width_px: u32) -> Option<f32> {
    pick(reported(), name, width_px)
}

fn pick(all: &[Reported], name: Option<&str>, width_px: u32) -> Option<f32> {
    if width_px == 0 {
        return None;
    }
    let found = name.and_then(|n| all.iter().find(|r| r.connector == n)).or(if all.len() == 1 { all.first() } else { None })?;
    Some(found.width_mm as f32 / width_px as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edid(dtd_mm: (u32, u32), basic_cm: (u8, u8)) -> Vec<u8> {
        let mut e = vec![0u8; 128];
        e[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        e[21] = basic_cm.0;
        e[22] = basic_cm.1;
        if dtd_mm.0 > 0 {
            e[54] = 0x02;
            e[55] = 0x3A; // a pixel clock
            e[66] = (dtd_mm.0 & 0xFF) as u8;
            e[67] = (dtd_mm.1 & 0xFF) as u8;
            e[68] = (((dtd_mm.0 >> 8) as u8) << 4) | (dtd_mm.1 >> 8) as u8;
        }
        e
    }

    #[test]
    fn reads_the_size_from_the_edid() {
        // a 27" 16:9 monitor: 597 x 336 mm
        assert_eq!(size_mm(&edid((597, 336), (60, 34))), Some((597, 336)));
        // no detailed timing: the size in cm
        assert_eq!(size_mm(&edid((0, 0), (60, 34))), Some((600, 340)));
        // a projector says nothing
        assert_eq!(size_mm(&edid((0, 0), (0, 0))), None);
        assert_eq!(size_mm(&[0u8; 64]), None);
    }

    #[test]
    fn finds_the_monitor_by_its_connector() {
        let all = [Reported { connector: "DP-1".into(), width_mm: 597 }, Reported { connector: "eDP-1".into(), width_mm: 344 }];
        let k = pick(&all, Some("eDP-1"), 1920).unwrap();
        assert!((k - 344.0 / 1920.0).abs() < 1e-6);
        // two screens and the name doesn't match (some X11 drivers name them differently): don't guess
        assert_eq!(pick(&all, Some("DisplayPort-0"), 2560), None);
        // one screen: that one
        assert!(pick(&all[..1], Some("DisplayPort-0"), 2560).is_some());
        assert_eq!(pick(&[], Some("DP-1"), 2560), None);
    }
}
