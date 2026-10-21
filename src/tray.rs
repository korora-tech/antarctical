//! StatusNotifierItem tray icon: today's date as the icon, today's agenda in the menu.
//! Runs on ksni's own thread; menu clicks come back to GPUI over a channel.

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{MenuItem, ToolTip};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Open,
    Refresh,
    Quit,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayContent {
    pub day: u8,
    /// e.g. "Wednesday 21 October".
    pub date_label: String,
    /// (time, title) for today's remaining events.
    pub agenda: Vec<(String, String)>,
    pub tooltip: String,
}

struct AntarcticalTray {
    content: TrayContent,
    actions: async_channel::Sender<TrayAction>,
}

impl AntarcticalTray {
    fn send(&self, action: TrayAction) {
        let _ = self.actions.send_blocking(action);
    }
}

impl ksni::Tray for AntarcticalTray {
    fn id(&self) -> String {
        "com.korora.Antarctical".into()
    }

    fn title(&self) -> String {
        "Antarctical".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [22, 32, 64]
            .into_iter()
            .map(|size| ksni::Icon {
                width: size,
                height: size,
                data: render_icon(self.content.day, size as usize),
            })
            .collect()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: format!("Antarctical · {}", self.content.date_label),
            description: self.content.tooltip.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayAction::Open);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: self.content.date_label.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
        ];
        if self.content.agenda.is_empty() {
            items.push(
                StandardItem {
                    label: "Nothing else today".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }
        for (time, title) in self.content.agenda.iter().take(8) {
            items.push(
                StandardItem {
                    // Underscores are mnemonic markers in SNI menus.
                    label: format!("{time}  {}", title.replace('_', "__")),
                    activate: Box::new(|t: &mut Self| t.send(TrayAction::Open)),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.extend([
            MenuItem::Separator,
            StandardItem {
                label: "Open Antarctical".into(),
                icon_name: "x-office-calendar".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayAction::Open)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Refresh".into(),
                icon_name: "view-refresh".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayAction::Refresh)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayAction::Quit)),
                ..Default::default()
            }
            .into(),
        ]);
        items
    }
}

/// Cheap to clone; `update` blocks briefly on D-Bus, so call it off the UI thread.
#[derive(Clone)]
pub struct Tray {
    handle: std::sync::Arc<Handle<AntarcticalTray>>,
}

impl Tray {
    /// Fails when there is no StatusNotifierWatcher (e.g. GNOME without the AppIndicator
    /// extension); the app then runs without a tray and quits when its window closes.
    pub fn spawn(
        content: TrayContent,
        actions: async_channel::Sender<TrayAction>,
    ) -> Result<Self, String> {
        AntarcticalTray { content, actions }
            .spawn()
            .map(|handle| Self {
                handle: std::sync::Arc::new(handle),
            })
            .map_err(|e| e.to_string())
    }

    pub fn update(&self, content: TrayContent) {
        self.handle.update(move |t| {
            if t.content != content {
                t.content = content;
            }
        });
    }

    pub fn shutdown(&self) {
        self.handle.shutdown().wait();
    }
}

// ----- icon ---------------------------------------------------------------------------------

/// 5×7 digits, one row per byte, MSB-first in the low 5 bits.
const DIGITS: [[u8; 7]; 10] = [
    [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
    [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
    [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
    [0x1f, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0e],
    [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
    [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
    [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
    [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
    [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
    [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
];

const ACCENT: [u8; 3] = [0x9c, 0x83, 0xff];
const PAPER: [u8; 3] = [0xf7, 0xf7, 0xfa];
const INK: [u8; 3] = [0x1b, 0x1d, 0x26];

/// A calendar page showing `day`, as ARGB32 in network byte order (what SNI expects).
pub fn render_icon(day: u8, size: usize) -> Vec<u8> {
    let mut px = vec![[0u8; 4]; size * size];
    let s = size as f32 / 64.0;
    let radius = 10.0 * s;
    let (x0, y0, x1, y1) = (4.0 * s, 6.0 * s, 60.0 * s, 60.0 * s);
    let header_bottom = 21.0 * s;
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if fx < x0 || fx > x1 || fy < y0 || fy > y1 {
                continue;
            }
            // Rounded corners.
            let cx = fx.clamp(x0 + radius, x1 - radius);
            let cy = fy.clamp(y0 + radius, y1 - radius);
            if (fx - cx).powi(2) + (fy - cy).powi(2) > radius * radius {
                continue;
            }
            let rgb = if fy < header_bottom { ACCENT } else { PAPER };
            px[y * size + x] = [0xff, rgb[0], rgb[1], rgb[2]];
        }
    }
    // Binder rings.
    for ring_x in [18.0, 46.0] {
        fill_rect(
            &mut px,
            size,
            (ring_x - 2.5) * s,
            2.0 * s,
            5.0 * s,
            9.0 * s,
            INK,
        );
    }
    // Digits, scaled to fit the page below the header.
    let digits: Vec<usize> = if day >= 10 {
        vec![(day / 10) as usize, (day % 10) as usize]
    } else {
        vec![day as usize]
    };
    let scale = 4.0 * s;
    let gap = 3.0 * s;
    let width = digits.len() as f32 * 5.0 * scale + (digits.len() as f32 - 1.0) * gap;
    let top = header_bottom + (y1 - header_bottom - 7.0 * scale) / 2.0;
    let mut left = (size as f32 - width) / 2.0;
    for d in digits {
        for (row, bits) in DIGITS[d % 10].iter().enumerate() {
            for col in 0..5 {
                if bits & (0x10 >> col) != 0 {
                    fill_rect(
                        &mut px,
                        size,
                        left + col as f32 * scale,
                        top + row as f32 * scale,
                        scale,
                        scale,
                        INK,
                    );
                }
            }
        }
        left += 5.0 * scale + gap;
    }
    px.into_iter().flatten().collect()
}

fn fill_rect(px: &mut [[u8; 4]], size: usize, x: f32, y: f32, w: f32, h: f32, rgb: [u8; 3]) {
    let (xa, ya) = (x.round().max(0.0) as usize, y.round().max(0.0) as usize);
    let (xb, yb) = (
        ((x + w).round() as usize).min(size),
        ((y + h).round() as usize).min(size),
    );
    for yy in ya..yb {
        for xx in xa..xb {
            px[yy * size + xx] = [0xff, rgb[0], rgb[1], rgb[2]];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_sizes_and_ink() {
        for size in [22, 32, 64] {
            let data = render_icon(21, size);
            assert_eq!(data.len(), size * size * 4);
            let ink = data
                .chunks(4)
                .filter(|p| p == &[0xff, INK[0], INK[1], INK[2]])
                .count();
            assert!(ink > 0, "digits drawn at {size}px");
        }
        // Corners stay transparent.
        assert_eq!(&render_icon(1, 64)[0..4], &[0, 0, 0, 0]);
    }
}
