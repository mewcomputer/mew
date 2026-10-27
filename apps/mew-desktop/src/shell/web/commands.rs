use mew_browser_host::BrowserRect;
use serde::{Deserialize, Serialize};

use super::super::helpers::browser_url_is_navigable;

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum HostCommand {
    Bootstrap {},
    BrowserUpdate {
        session_id: String,
        tab_id: String,
        url: String,
        rect: ViewportRect,
        visible: bool,
    },
    BrowserClose {
        session_id: String,
        tab_id: String,
    },
    SetTheme {
        #[serde(flatten)]
        preferences: ThemePreferences,
    },
}

impl HostCommand {
    pub(super) fn parse(input: &str) -> Result<Self, String> {
        if input.len() > 16_384 {
            return Err("native command exceeds 16 KiB".into());
        }
        let command: Self = serde_json::from_str(input).map_err(|error| error.to_string())?;
        match &command {
            Self::BrowserUpdate {
                session_id, tab_id, ..
            }
            | Self::BrowserClose { session_id, tab_id }
                if session_id.trim().is_empty() || tab_id.trim().is_empty() =>
            {
                return Err("browser commands require a session and tab identity".into());
            }
            _ => {}
        }
        if let Self::BrowserUpdate {
            url, rect, visible, ..
        } = &command
        {
            if !browser_url_is_navigable(url) || url.chars().any(char::is_whitespace) {
                return Err(
                    "browser navigation only supports about:blank, http, and https URLs".into(),
                );
            }
            if ![rect.x, rect.y, rect.width, rect.height]
                .into_iter()
                .all(f64::is_finite)
                || rect.width < 0.
                || rect.height < 0.
                || (*visible && (rect.width == 0. || rect.height == 0.))
            {
                return Err(
                    "browser bounds must be finite and visible surfaces need positive dimensions"
                        .into(),
                );
            }
        }
        Ok(command)
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ViewportRect {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
}

impl ViewportRect {
    pub(super) fn in_viewport(self, viewport: BrowserRect, visible: bool) -> BrowserRect {
        if ![
            self.x,
            self.y,
            self.width,
            self.height,
            viewport.width,
            viewport.height,
        ]
        .into_iter()
        .all(f64::is_finite)
            || viewport.width <= 0.
            || viewport.height <= 0.
        {
            return BrowserRect {
                width: 0.,
                height: 0.,
                visible: false,
                ..viewport
            };
        }
        let left = self.x.clamp(0., viewport.width);
        let top = self.y.clamp(0., viewport.height);
        let right = (self.x + self.width).clamp(0., viewport.width);
        let bottom = (self.y + self.height).clamp(0., viewport.height);
        let width = (right - left).max(0.);
        let height = (bottom - top).max(0.);
        BrowserRect {
            x: viewport.x + left,
            y: viewport.y + viewport.height - bottom,
            width,
            height,
            visible: visible && viewport.visible && width > 0. && height > 0.,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ThemePreferences {
    pub(super) theme_mode: ThemeMode,
    pub(super) light_theme: String,
    pub(super) dark_theme: String,
}

impl ThemePreferences {
    pub(super) fn apply_to(&self, state: &mut mew_config::State) {
        state.desktop_theme_mode = match self.theme_mode {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
        .into();
        state.desktop_light_theme.clone_from(&self.light_theme);
        state.desktop_dark_theme.clone_from(&self.dark_theme);
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ThemeMode {
    System,
    Light,
    Dark,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(url: &str, width: f64, visible: bool) -> String {
        serde_json::json!({
            "type": "browser_update",
            "sessionId": "chat",
            "tabId": "browser",
            "url": url,
            "rect": { "x": -20, "y": 10, "width": width, "height": 100 },
            "visible": visible,
        })
        .to_string()
    }

    #[test]
    fn browser_commands_reject_privileged_urls_and_invalid_visible_bounds() {
        for url in [
            "file:///tmp/private",
            "javascript:alert(1)",
            "data:text/html,hello",
            "https://example.com/ has-space",
        ] {
            assert!(HostCommand::parse(&update(url, 100., true)).is_err());
        }
        for url in [
            "about:blank",
            "http://localhost:3000/",
            "https://example.com/",
        ] {
            assert!(HostCommand::parse(&update(url, 100., true)).is_ok());
        }
        assert!(HostCommand::parse(&update("about:blank", 0., true)).is_err());
        assert!(HostCommand::parse(&update("about:blank", 0., false)).is_ok());
        assert!(HostCommand::parse(&update("about:blank", -1., false)).is_err());
        assert!(HostCommand::parse(
            r#"{"type":"browser_close","sessionId":"  ","tabId":"browser"}"#
        )
        .is_err());
    }

    #[test]
    fn command_schema_rejects_nested_unknown_fields_and_nonfinite_coordinates() {
        for input in [
            r#"{"type":"browser_update","sessionId":"chat","tabId":"browser","url":"about:blank","rect":{"x":0,"y":0,"width":100,"height":100,"extra":true},"visible":true}"#,
            r#"{"type":"browser_update","sessionId":"chat","tabId":"browser","url":"about:blank","rect":{"x":1e999,"y":0,"width":100,"height":100},"visible":true}"#,
            r#"{"type":"browser_update","sessionId":"chat","tabId":"browser","url":"about:blank","rect":{"x":0,"y":0,"width":100,"height":100},"visible":true,"extra":true}"#,
        ] {
            assert!(HostCommand::parse(input).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn flattened_theme_preferences_use_the_bootstrap_field_names_and_reject_extras() {
        let command = HostCommand::parse(
            r#"{"type":"set_theme","themeMode":"dark","lightTheme":"light","darkTheme":"nord"}"#,
        )
        .unwrap();
        let HostCommand::SetTheme { preferences } = command else {
            panic!("expected theme preferences");
        };
        assert_eq!(
            serde_json::to_value(&preferences).unwrap(),
            serde_json::json!({
                "themeMode": "dark", "lightTheme": "light", "darkTheme": "nord",
            })
        );
        for input in [
            r#"{"type":"set_theme","themeMode":"automatic","lightTheme":"light","darkTheme":"nord"}"#,
            r#"{"type":"set_theme","themeMode":"system","lightTheme":"light","darkTheme":"nord","extra":true}"#,
        ] {
            assert!(HostCommand::parse(input).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn clipping_translates_negative_offsets_and_respects_hidden_viewports() {
        let viewport = BrowserRect {
            x: 15.,
            y: 32.,
            width: 1000.,
            height: 700.,
            visible: true,
        };
        let rect = ViewportRect {
            x: -20.,
            y: -10.,
            width: 100.,
            height: 100.,
        }
        .in_viewport(viewport, true);
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (15., 642., 80., 90.)
        );
        assert!(rect.visible);
        let hidden = ViewportRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        }
        .in_viewport(
            BrowserRect {
                visible: false,
                ..viewport
            },
            true,
        );
        assert!(!hidden.visible);
    }
}
