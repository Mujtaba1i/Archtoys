//! Which color formats are shown and in which order. The first enabled one is
//! the big value on the color card and what Auto Copy copies.

use crate::color::ColorField;
use serde::{Deserialize, Serialize};

/// How a format is saved in config.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatEntry {
    pub key: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatList {
    items: Vec<(ColorField, bool)>,
}

impl Default for FormatList {
    fn default() -> Self {
        Self {
            items: ColorField::ALL.iter().map(|f| (*f, f.on_by_default())).collect(),
        }
    }
}

impl FormatList {
    /// From config.json. Unknown keys are ignored; formats added in a newer
    /// version (missing from the file) are appended with their default state.
    pub fn from_saved(saved: &[FormatEntry]) -> Self {
        if saved.is_empty() {
            return Self::default();
        }
        let mut items: Vec<(ColorField, bool)> = vec![];
        for entry in saved {
            if let Some(field) = ColorField::from_ui_label(&entry.key) {
                if !items.iter().any(|(f, _)| *f == field) {
                    items.push((field, entry.enabled));
                }
            }
        }
        for field in ColorField::ALL {
            if !items.iter().any(|(f, _)| *f == field) {
                items.push((field, field.on_by_default()));
            }
        }
        let mut list = Self { items };
        if list.enabled().is_empty() {
            list.set_state(ColorField::Hex, true);
        }
        list
    }

    pub fn to_saved(&self) -> Vec<FormatEntry> {
        self.items
            .iter()
            .map(|(f, on)| FormatEntry {
                key: f.label().to_string(),
                enabled: *on,
            })
            .collect()
    }

    /// All formats with their on/off state, in order.
    pub fn items(&self) -> &[(ColorField, bool)] {
        &self.items
    }

    /// The enabled formats, in order.
    pub fn enabled(&self) -> Vec<ColorField> {
        self.items.iter().filter(|(_, on)| *on).map(|(f, _)| *f).collect()
    }

    /// The first enabled format: shown big on the color card, and what Auto
    /// Copy copies.
    pub fn auto_copy_field(&self) -> ColorField {
        self.enabled().first().copied().unwrap_or(ColorField::Hex)
    }

    fn set_state(&mut self, field: ColorField, on: bool) {
        if let Some(item) = self.items.iter_mut().find(|(f, _)| *f == field) {
            item.1 = on;
        }
    }

    /// Turns a format on or off. At least one format must stay on, so there is
    /// always something on the color card to copy.
    pub fn set_enabled(&mut self, field: ColorField, on: bool) -> Result<(), &'static str> {
        let currently_on = self.items.iter().any(|(f, s)| *f == field && *s);
        if !on && currently_on && self.enabled().len() <= 1 {
            return Err("Keep at least one color format turned on");
        }
        self.set_state(field, on);
        Ok(())
    }

    /// Moves a format up (delta -1) or down (+1). Returns whether it moved.
    pub fn move_by(&mut self, field: ColorField, delta: i32) -> bool {
        let Some(index) = self.items.iter().position(|(f, _)| *f == field) else {
            return false;
        };
        let target = index as i32 + delta;
        if target < 0 || target >= self.items.len() as i32 {
            return false;
        }
        self.items.swap(index, target as usize);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let list = FormatList::default();
        assert_eq!(
            list.enabled(),
            vec![ColorField::Hex, ColorField::Rgb, ColorField::Hsl, ColorField::Hsv]
        );
        assert_eq!(list.auto_copy_field(), ColorField::Hex);
    }

    #[test]
    fn auto_copy_follows_the_order() {
        let mut list = FormatList::default();
        list.set_enabled(ColorField::Oklch, true).unwrap();
        while list.items()[0].0 != ColorField::Oklch {
            assert!(list.move_by(ColorField::Oklch, -1));
        }
        assert_eq!(list.auto_copy_field(), ColorField::Oklch);
        assert!(!list.move_by(ColorField::Oklch, -1)); // already first
    }

    #[test]
    fn last_format_cannot_be_turned_off() {
        let mut list = FormatList::default();
        for f in [ColorField::Hex, ColorField::Rgb, ColorField::Hsl] {
            list.set_enabled(f, false).unwrap();
        }
        assert!(list.set_enabled(ColorField::Hsv, false).is_err());
        assert_eq!(list.enabled(), vec![ColorField::Hsv]);
    }

    #[test]
    fn saved_lists_load_and_new_formats_are_added() {
        let saved = vec![
            FormatEntry { key: "RGB".into(), enabled: true },
            FormatEntry { key: "HEX".into(), enabled: false },
            FormatEntry { key: "NAME".into(), enabled: true }, // no longer a format: ignored
            FormatEntry { key: "FUTURE".into(), enabled: true }, // unknown: ignored
        ];
        let list = FormatList::from_saved(&saved);
        assert_eq!(list.items()[0], (ColorField::Rgb, true));
        assert_eq!(list.items()[1], (ColorField::Hex, false));
        assert_eq!(list.items().len(), ColorField::ALL.len());
        assert_eq!(FormatList::from_saved(&list.to_saved()), list);
    }

    #[test]
    fn a_saved_list_with_nothing_copyable_gets_hex_back() {
        let saved = vec![FormatEntry { key: "HEX".into(), enabled: false }];
        let all_off: Vec<FormatEntry> = ColorField::ALL
            .iter()
            .map(|f| FormatEntry { key: f.label().into(), enabled: false })
            .collect();
        assert!(FormatList::from_saved(&saved).enabled().len() > 1);
        assert_eq!(FormatList::from_saved(&all_off).enabled(), vec![ColorField::Hex]);
    }
}
