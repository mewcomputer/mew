use std::borrow::Cow;

use anyhow::Result;
use gpui::{svg, AssetSource, Hsla, IntoElement, Pixels, SharedString, Styled};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TablerIcon {
    Archive,
    ArrowLeft,
    ArrowUp,
    Browser,
    Bulb,
    ChevronDown,
    ChevronRight,
    Dots,
    ExternalLink,
    File,
    FileCode,
    Folder,
    GitBranch,
    GripVertical,
    MessageCircle,
    PanelLeft,
    PanelRight,
    Paperclip,
    Pencil,
    Pin,
    Plus,
    Search,
    Settings,
    ShieldLock,
    SlidersHorizontal,
    Square,
    Terminal2,
    UserCircle,
    X,
}

impl TablerIcon {
    const ALL: [Self; 29] = [
        Self::Archive,
        Self::ArrowLeft,
        Self::ArrowUp,
        Self::Browser,
        Self::Bulb,
        Self::ChevronDown,
        Self::ChevronRight,
        Self::Dots,
        Self::ExternalLink,
        Self::File,
        Self::FileCode,
        Self::Folder,
        Self::GitBranch,
        Self::GripVertical,
        Self::MessageCircle,
        Self::PanelLeft,
        Self::PanelRight,
        Self::Paperclip,
        Self::Pencil,
        Self::Pin,
        Self::Plus,
        Self::Search,
        Self::Settings,
        Self::ShieldLock,
        Self::SlidersHorizontal,
        Self::Square,
        Self::Terminal2,
        Self::UserCircle,
        Self::X,
    ];

    const fn asset_path(self) -> &'static str {
        match self {
            Self::Archive => "icons/archive.svg",
            Self::ArrowLeft => "icons/arrow-left.svg",
            Self::ArrowUp => "icons/arrow-up.svg",
            Self::Browser => "icons/browser.svg",
            Self::Bulb => "icons/bulb.svg",
            Self::ChevronDown => "icons/chevron-down.svg",
            Self::ChevronRight => "icons/chevron-right.svg",
            Self::Dots => "icons/dots.svg",
            Self::ExternalLink => "icons/external-link.svg",
            Self::File => "icons/file.svg",
            Self::FileCode => "icons/file-code.svg",
            Self::Folder => "icons/folder.svg",
            Self::GitBranch => "icons/git-branch.svg",
            Self::GripVertical => "icons/grip-vertical.svg",
            Self::MessageCircle => "icons/message-circle.svg",
            Self::PanelLeft => "icons/panel-left.svg",
            Self::PanelRight => "icons/panel-right.svg",
            Self::Paperclip => "icons/paperclip.svg",
            Self::Pencil => "icons/pencil.svg",
            Self::Pin => "icons/pin.svg",
            Self::Plus => "icons/plus.svg",
            Self::Search => "icons/search.svg",
            Self::Settings => "icons/settings.svg",
            Self::ShieldLock => "icons/shield-lock.svg",
            Self::SlidersHorizontal => "icons/sliders-horizontal.svg",
            Self::Square => "icons/square.svg",
            Self::Terminal2 => "icons/terminal-2.svg",
            Self::UserCircle => "icons/user-circle.svg",
            Self::X => "icons/x.svg",
        }
    }

    const fn body(self) -> &'static str {
        match self {
            Self::Archive => {
                "<path d=\"M3 4m0 2a2 2 0 0 1 2 -2h14a2 2 0 0 1 2 2v0a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2z\"/><path d=\"M5 8v10a2 2 0 0 0 2 2h10a2 2 0 0 0 2 -2v-10\"/><path d=\"M10 12l4 0\"/>"
            }
            Self::ArrowLeft => {
                "<path d=\"M5 12l6 -6\"/><path d=\"M5 12l6 6\"/><path d=\"M5 12h14\"/>"
            }
            Self::ArrowUp => "<path d=\"M12 5l0 14\"/><path d=\"M18 11l-6 -6l-6 6\"/>",
            Self::Browser => {
                "<rect x=\"4\" y=\"4\" width=\"16\" height=\"16\" rx=\"2\"/><path d=\"M4 9h16\"/><path d=\"M8 4v5\"/>"
            }
            Self::Bulb => {
                "<path d=\"M3 12h1m8 -9v1m8 8h1m-15.4 -6.4l.7 .7m12.1 -.7l-.7 .7\"/><path d=\"M9 16a5 5 0 1 1 6 0a3.5 3.5 0 0 0 -1 3a2 2 0 0 1 -4 0a3.5 3.5 0 0 0 -1 -3\"/><path d=\"M9.7 17l4.6 0\"/>"
            }
            Self::ChevronDown => "<path d=\"M6 9l6 6l6 -6\"/>",
            Self::ChevronRight => "<path d=\"M9 6l6 6l-6 6\"/>",
            Self::Dots => {
                "<circle cx=\"5\" cy=\"12\" r=\"1\"/><circle cx=\"12\" cy=\"12\" r=\"1\"/><circle cx=\"19\" cy=\"12\" r=\"1\"/>"
            }
            Self::ExternalLink => {
                "<path d=\"M11 7h-4a2 2 0 0 0 -2 2v8a2 2 0 0 0 2 2h8a2 2 0 0 0 2 -2v-4\"/><path d=\"M10 14l9 -9\"/><path d=\"M15 5h4v4\"/>"
            }
            Self::File => {
                "<path d=\"M14 3v4a1 1 0 0 0 1 1h4\"/><path d=\"M17 21h-10a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2z\"/>"
            }
            Self::FileCode => {
                "<path d=\"M14 3v4a1 1 0 0 0 1 1h4\"/><path d=\"M17 21h-10a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2h7l5 5v11a2 2 0 0 1 -2 2z\"/><path d=\"M10 13l-2 2l2 2\"/><path d=\"M14 13l2 2l-2 2\"/>"
            }
            Self::Folder => {
                "<path d=\"M5 4h4l3 3h7a2 2 0 0 1 2 2v7a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-10a2 2 0 0 1 2 -2\"/>"
            }
            Self::GitBranch => {
                "<circle cx=\"6\" cy=\"6\" r=\"2\"/><circle cx=\"18\" cy=\"18\" r=\"2\"/><circle cx=\"6\" cy=\"18\" r=\"2\"/><path d=\"M6 8v8\"/><path d=\"M18 16a8 8 0 0 0 -8 -8h-2\"/>"
            }
            Self::GripVertical => {
                "<circle cx=\"8\" cy=\"6\" r=\"1\"/><circle cx=\"16\" cy=\"6\" r=\"1\"/><circle cx=\"8\" cy=\"12\" r=\"1\"/><circle cx=\"16\" cy=\"12\" r=\"1\"/><circle cx=\"8\" cy=\"18\" r=\"1\"/><circle cx=\"16\" cy=\"18\" r=\"1\"/>"
            }
            Self::MessageCircle => {
                "<path d=\"M3 20l1.3 -3.9a9 9 0 1 1 3.7 3.7l-4 1.2\"/><path d=\"M8 12h.01\"/><path d=\"M12 12h.01\"/><path d=\"M16 12h.01\"/>"
            }
            Self::PanelLeft => {
                "<rect x=\"3\" y=\"4\" width=\"18\" height=\"16\" rx=\"2\"/><path d=\"M9 4v16\"/>"
            }
            Self::PanelRight => {
                "<rect x=\"3\" y=\"4\" width=\"18\" height=\"16\" rx=\"2\"/><path d=\"M15 4v16\"/>"
            }
            Self::Paperclip => {
                "<path d=\"M15 7l-6.5 6.5a2.121 2.121 0 0 0 3 3l6.5 -6.5a4.243 4.243 0 0 0 -6 -6l-6.5 6.5a6.364 6.364 0 0 0 9 9l6.5 -6.5\"/>"
            }
            Self::Pencil => {
                "<path d=\"M4 20h4l10.5 -10.5a2.828 2.828 0 1 0 -4 -4l-10.5 10.5v4\"/><path d=\"M13.5 6.5l4 4\"/>"
            }
            Self::Pin => {
                "<path d=\"M15 4.5l-4 4l-4 1l-1 1l4.5 4.5l1 -4l4 -4z\"/><path d=\"M9 15l-4.5 4.5\"/><path d=\"M14.5 4l5.5 5.5\"/>"
            }
            Self::Plus => "<path d=\"M12 5l0 14\"/><path d=\"M5 12l14 0\"/>",
            Self::Search => "<circle cx=\"10\" cy=\"10\" r=\"7\"/><path d=\"M21 21l-6 -6\"/>",
            Self::Settings => {
                "<path d=\"M10.325 4.317c.426 -1.756 2.924 -1.756 3.35 0a1.724 1.724 0 0 0 2.573 1.066c1.543 -.94 3.543 .826 2.588 2.37a1.724 1.724 0 0 0 1.065 2.572c1.756 .426 1.756 2.924 0 3.35a1.724 1.724 0 0 0 -1.066 2.573c.94 1.543 -.826 3.543 -2.37 2.588a1.724 1.724 0 0 0 -2.572 1.065c-.426 1.756 -2.924 1.756 -3.35 0a1.724 1.724 0 0 0 -2.573 -1.066c-1.543 .94 -3.543 -.826 -2.588 -2.37a1.724 1.724 0 0 0 -1.065 -2.572c-1.756 -.426 -1.756 -2.924 0 -3.35a1.724 1.724 0 0 0 1.066 -2.573c-.94 -1.543 .826 -3.543 2.37 -2.588a1.724 1.724 0 0 0 2.572 -1.065\"/><circle cx=\"12\" cy=\"12\" r=\"3\"/>"
            }
            Self::ShieldLock => {
                "<path d=\"M12 3a12 12 0 0 0 8.5 3a12 12 0 0 1 -8.5 15a12 12 0 0 1 -8.5 -15a12 12 0 0 0 8.5 -3\"/><path d=\"M12 11m-1 0a1 1 0 1 0 2 0a1 1 0 1 0 -2 0\"/><path d=\"M12 12l0 2.5\"/>"
            }
            Self::SlidersHorizontal => {
                "<line x1=\"4\" y1=\"6\" x2=\"20\" y2=\"6\"/><line x1=\"4\" y1=\"12\" x2=\"20\" y2=\"12\"/><line x1=\"4\" y1=\"18\" x2=\"20\" y2=\"18\"/><circle cx=\"8\" cy=\"6\" r=\"2\"/><circle cx=\"16\" cy=\"12\" r=\"2\"/><circle cx=\"10\" cy=\"18\" r=\"2\"/>"
            }
            Self::Square => "<rect x=\"5\" y=\"5\" width=\"14\" height=\"14\" rx=\"2\"/>",
            Self::Terminal2 => {
                "<path d=\"M4 5a2 2 0 0 1 2 -2h12a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-12a2 2 0 0 1 -2 -2z\"/><path d=\"M8 9l3 3l-3 3\"/><path d=\"M13 15l3 0\"/>"
            }
            Self::UserCircle => {
                "<circle cx=\"12\" cy=\"12\" r=\"9\"/><circle cx=\"12\" cy=\"10\" r=\"3\"/><path d=\"M6 19a6 6 0 0 1 12 0\"/>"
            }
            Self::X => "<path d=\"M18 6l-12 12\"/><path d=\"M6 6l12 12\"/>",
        }
    }

    fn document(self) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            self.body()
        )
    }
}

pub fn tabler_icon(icon: TablerIcon, color: impl Into<Hsla>, size: Pixels) -> impl IntoElement {
    svg()
        .path(icon.asset_path())
        .size(size)
        .flex_none()
        .text_color(color.into())
}

pub struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let Some(icon) = TablerIcon::ALL
            .into_iter()
            .find(|icon| icon.asset_path() == path)
        else {
            return Ok(None);
        };
        Ok(Some(Cow::Owned(icon.document().into_bytes())))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_a_loadable_tabler_document() {
        let assets = IconAssets;
        for icon in TablerIcon::ALL {
            let path = icon.asset_path();
            let document = assets
                .load(path)
                .expect("icon assets should load")
                .expect("registered icon should have an asset");
            let document = String::from_utf8(document.into_owned()).expect("svg is utf8");
            assert!(document.starts_with("<svg "));
            assert!(document.contains("currentColor"));
        }
    }
}
