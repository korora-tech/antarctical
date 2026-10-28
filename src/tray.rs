//! StatusNotifierItem tray: a fixed penguin icon and today's agenda in the menu.
//! Runs on ksni's own thread; menu clicks come back to GPUI over a channel.

use std::sync::LazyLock;

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{MenuItem, ToolTip};

use crate::branding;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    TogglePopover { x: i32, y: i32 },
    OpenMain,
    Refresh,
    Quit,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayContent {
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
        branding::APP_ID.into()
    }

    fn title(&self) -> String {
        "Antarctical".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        TRAY_ICONS
            .iter()
            .map(|(size, data)| ksni::Icon {
                width: *size,
                height: *size,
                data: data.clone(),
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

    fn activate(&mut self, x: i32, y: i32) {
        self.send(TrayAction::TogglePopover { x, y });
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
                    activate: Box::new(|t: &mut Self| t.send(TrayAction::OpenMain)),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.extend([
            MenuItem::Separator,
            StandardItem {
                label: "Open full calendar".into(),
                icon_name: branding::APP_ID.into(),
                activate: Box::new(|t: &mut Self| t.send(TrayAction::OpenMain)),
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

// Decode and resize once. Agenda updates never redraw or change the penguin.
static TRAY_ICONS: LazyLock<Vec<(i32, Vec<u8>)>> = LazyLock::new(|| {
    [22, 32, 64]
        .into_iter()
        .map(|size| (size, render_icon(size as u32)))
        .collect()
});

/// The compact penguin as straight-alpha ARGB32 in network byte order for SNI.
fn render_icon(size: u32) -> Vec<u8> {
    let image = image::load_from_memory_with_format(branding::ICON_PNG, image::ImageFormat::Png)
        .expect("the embedded penguin icon must be a valid PNG")
        .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    image
        .pixels()
        .flat_map(|p| [p[3], p[0], p[1], p[2]])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ksni::Tray as _;

    #[test]
    fn penguin_pixmaps_have_the_requested_size_and_alpha() {
        for size in [22, 32, 64] {
            let data = render_icon(size);
            assert_eq!(data.len(), (size * size * 4) as usize);
            assert_eq!(&data[0..4], &[0, 0, 0, 0]);
            assert!(data.as_chunks::<4>().0.iter().any(|p| p[0] == 255));
        }
        // A solid navy pixel on the head verifies the SNI alpha/red/green/blue order.
        let data = render_icon(512);
        let head = (80 * 512 + 256) * 4;
        assert_eq!(&data[head..head + 4], &[255, 0x28, 0x4f, 0x73]);
    }

    #[test]
    fn date_and_agenda_changes_do_not_change_the_tray_icon() {
        let (actions, _) = async_channel::unbounded();
        let mut tray = AntarcticalTray {
            content: TrayContent {
                date_label: "Wednesday 28 October".into(),
                ..Default::default()
            },
            actions,
        };
        let before = tray.icon_pixmap();
        tray.content.date_label = "Thursday 29 October".into();
        tray.content
            .agenda
            .push(("09:00".into(), "Planning".into()));
        let after = tray.icon_pixmap();
        assert_eq!(before.len(), 3);
        assert_eq!(before.len(), after.len());
        for (before, after) in before.iter().zip(&after) {
            assert_eq!(before.width, after.width);
            assert_eq!(before.height, after.height);
            assert_eq!(before.data, after.data);
        }
        assert!(tray.tool_tip().title.contains("Thursday 29 October"));
    }
}
