#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(clippy::all)]
#![allow(rustdoc::all)]

mod bindings;

use std::ops::Deref;

pub use bindings::*;

/// Initialize a "sized" FFI object.
#[macro_export]
macro_rules! sized {
    ($ty:ty) => {{
        let mut t = <$ty as ::std::default::Default>::default();
        t.size = ::std::mem::size_of::<$ty>();
        t
    }};
}

impl<S> From<S> for bindings::String
where
    S: Deref<Target = str>,
{
    fn from(value: S) -> Self {
        Self {
            ptr: value.as_ptr(),
            len: value.len(),
        }
    }
}

impl bindings::String {
    /// Borrow the string as raw bytes. A null pointer is treated as an empty
    /// string; the C API does not promise a non-null pointer for zero-length
    /// strings, and `slice::from_raw_parts` requires one even at length zero.
    ///
    /// # Safety
    ///
    /// The caller must uphold that the associated lifetime is valid
    /// with the given context behind the FFI string.
    pub unsafe fn to_bytes<'a>(self) -> &'a [u8] {
        if self.ptr.is_null() {
            return &[];
        }
        // SAFETY: To be upheld by caller
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// # Safety
    ///
    /// The caller must uphold that the associated lifetime is valid
    /// with the given context behind the FFI string, and that it contains
    /// valid UTF-8 data.
    pub unsafe fn to_str<'a>(self) -> &'a str {
        // SAFETY: To be upheld by caller
        unsafe { std::str::from_utf8_unchecked(self.to_bytes()) }
    }
}

/// Canonical list of exported `libghostty-vt` C functions represented by checked-in bindings.
pub const EXPORTED_API_SYMBOLS: &[&str] = &[
    "ghostty_alloc",
    "ghostty_build_info",
    "ghostty_cell_get",
    "ghostty_cell_get_multi",
    "ghostty_color_contrast",
    "ghostty_color_luminance",
    "ghostty_color_palette_default",
    "ghostty_color_palette_generate",
    "ghostty_color_parse",
    "ghostty_color_parse_palette_entry",
    "ghostty_color_parse_x11",
    "ghostty_color_perceived_luminance",
    "ghostty_color_rgb_get",
    "ghostty_color_scheme_report_encode",
    "ghostty_color_x11_name_count",
    "ghostty_color_x11_names",
    "ghostty_focus_encode",
    "ghostty_formatter_format_alloc",
    "ghostty_formatter_format_buf",
    "ghostty_formatter_free",
    "ghostty_formatter_terminal_new",
    "ghostty_free",
    "ghostty_grid_ref_cell",
    "ghostty_grid_ref_graphemes",
    "ghostty_grid_ref_hyperlink_uri",
    "ghostty_grid_ref_row",
    "ghostty_grid_ref_style",
    "ghostty_key_encoder_encode",
    "ghostty_key_encoder_free",
    "ghostty_key_encoder_new",
    "ghostty_key_encoder_setopt",
    "ghostty_key_encoder_setopt_from_terminal",
    "ghostty_key_event_free",
    "ghostty_key_event_get_action",
    "ghostty_key_event_get_composing",
    "ghostty_key_event_get_consumed_mods",
    "ghostty_key_event_get_key",
    "ghostty_key_event_get_mods",
    "ghostty_key_event_get_unshifted_codepoint",
    "ghostty_key_event_get_utf8",
    "ghostty_key_event_new",
    "ghostty_key_event_set_action",
    "ghostty_key_event_set_composing",
    "ghostty_key_event_set_consumed_mods",
    "ghostty_key_event_set_key",
    "ghostty_key_event_set_mods",
    "ghostty_key_event_set_unshifted_codepoint",
    "ghostty_key_event_set_utf8",
    "ghostty_kitty_graphics_get",
    "ghostty_kitty_graphics_image",
    "ghostty_kitty_graphics_image_get",
    "ghostty_kitty_graphics_image_get_multi",
    "ghostty_kitty_graphics_placement_get",
    "ghostty_kitty_graphics_placement_get_multi",
    "ghostty_kitty_graphics_placement_grid_size",
    "ghostty_kitty_graphics_placement_iterator_free",
    "ghostty_kitty_graphics_placement_iterator_new",
    "ghostty_kitty_graphics_placement_iterator_set",
    "ghostty_kitty_graphics_placement_next",
    "ghostty_kitty_graphics_placement_pixel_size",
    "ghostty_kitty_graphics_placement_rect",
    "ghostty_kitty_graphics_placement_render_info",
    "ghostty_kitty_graphics_placement_source_rect",
    "ghostty_kitty_graphics_placement_viewport_pos",
    "ghostty_mode_report_encode",
    "ghostty_mouse_encoder_encode",
    "ghostty_mouse_encoder_free",
    "ghostty_mouse_encoder_new",
    "ghostty_mouse_encoder_reset",
    "ghostty_mouse_encoder_setopt",
    "ghostty_mouse_encoder_setopt_from_terminal",
    "ghostty_mouse_event_clear_button",
    "ghostty_mouse_event_free",
    "ghostty_mouse_event_get_action",
    "ghostty_mouse_event_get_button",
    "ghostty_mouse_event_get_mods",
    "ghostty_mouse_event_get_position",
    "ghostty_mouse_event_new",
    "ghostty_mouse_event_set_action",
    "ghostty_mouse_event_set_button",
    "ghostty_mouse_event_set_mods",
    "ghostty_mouse_event_set_position",
    "ghostty_osc_command_data",
    "ghostty_osc_command_type",
    "ghostty_osc_end",
    "ghostty_osc_free",
    "ghostty_osc_new",
    "ghostty_osc_next",
    "ghostty_osc_reset",
    "ghostty_paste_encode",
    "ghostty_paste_is_safe",
    "ghostty_render_state_begin_update",
    "ghostty_render_state_colors_get",
    "ghostty_render_state_end_update",
    "ghostty_render_state_free",
    "ghostty_render_state_get",
    "ghostty_render_state_get_multi",
    "ghostty_render_state_new",
    "ghostty_render_state_row_cells_free",
    "ghostty_render_state_row_cells_get",
    "ghostty_render_state_row_cells_get_multi",
    "ghostty_render_state_row_cells_new",
    "ghostty_render_state_row_cells_next",
    "ghostty_render_state_row_cells_select",
    "ghostty_render_state_row_get",
    "ghostty_render_state_row_get_multi",
    "ghostty_render_state_row_iterator_free",
    "ghostty_render_state_row_iterator_new",
    "ghostty_render_state_row_iterator_next",
    "ghostty_render_state_row_set",
    "ghostty_render_state_set",
    "ghostty_render_state_update",
    "ghostty_row_get",
    "ghostty_row_get_multi",
    "ghostty_selection_gesture_event",
    "ghostty_selection_gesture_event_free",
    "ghostty_selection_gesture_event_new",
    "ghostty_selection_gesture_event_set",
    "ghostty_selection_gesture_free",
    "ghostty_selection_gesture_get",
    "ghostty_selection_gesture_get_multi",
    "ghostty_selection_gesture_new",
    "ghostty_selection_gesture_reset",
    "ghostty_sgr_attribute_tag",
    "ghostty_sgr_attribute_value",
    "ghostty_sgr_free",
    "ghostty_sgr_new",
    "ghostty_sgr_next",
    "ghostty_sgr_reset",
    "ghostty_sgr_set_params",
    "ghostty_sgr_unknown_full",
    "ghostty_sgr_unknown_partial",
    "ghostty_size_report_encode",
    "ghostty_style_default",
    "ghostty_style_is_default",
    "ghostty_sys_log_stderr",
    "ghostty_sys_set",
    "ghostty_terminal_compress",
    "ghostty_terminal_compression_activity",
    "ghostty_terminal_free",
    "ghostty_terminal_get",
    "ghostty_terminal_get_multi",
    "ghostty_terminal_grid_ref",
    "ghostty_terminal_grid_ref_track",
    "ghostty_terminal_mode_get",
    "ghostty_terminal_mode_set",
    "ghostty_terminal_new",
    "ghostty_terminal_point_from_grid_ref",
    "ghostty_terminal_reset",
    "ghostty_terminal_resize",
    "ghostty_terminal_scroll_viewport",
    "ghostty_terminal_select_all",
    "ghostty_terminal_select_line",
    "ghostty_terminal_select_output",
    "ghostty_terminal_select_word",
    "ghostty_terminal_select_word_between",
    "ghostty_terminal_selection_adjust",
    "ghostty_terminal_selection_contains",
    "ghostty_terminal_selection_equal",
    "ghostty_terminal_selection_format_alloc",
    "ghostty_terminal_selection_format_buf",
    "ghostty_terminal_selection_order",
    "ghostty_terminal_selection_ordered",
    "ghostty_terminal_set",
    "ghostty_terminal_vt_write",
    "ghostty_tracked_grid_ref_free",
    "ghostty_tracked_grid_ref_has_value",
    "ghostty_tracked_grid_ref_point",
    "ghostty_tracked_grid_ref_set",
    "ghostty_tracked_grid_ref_snapshot",
    "ghostty_type_json",
    "ghostty_unicode_codepoint_width",
    "ghostty_unicode_grapheme_width",
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::EXPORTED_API_SYMBOLS;

    fn parse_binding_symbols(input: &str) -> BTreeSet<String> {
        input
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if !line.starts_with("pub fn ghostty_") {
                    return None;
                }

                let start = "pub fn ".len();
                let rest = &line[start..];
                let end = rest.find('(')?;
                Some(rest[..end].to_owned())
            })
            .collect()
    }

    fn parse_header_symbols(input: &str) -> BTreeSet<String> {
        input
            .lines()
            .filter(|line| !line.trim_start().starts_with("static "))
            .flat_map(|line| line.split("ghostty_").skip(1))
            .filter_map(|rest| {
                let end = rest.find('(')?;
                let name = rest[..end].trim();
                name.chars()
                    .all(|char| char.is_ascii_alphanumeric() || char == '_')
                    .then(|| format!("ghostty_{name}"))
            })
            .collect()
    }

    #[test]
    fn exported_manifest_matches_native_headers() {
        let include = std::path::PathBuf::from(env!("OUT_DIR")).join("ghostty-install/include");
        let headers = std::process::Command::new("cc")
            .args(["-E", "-P", "-x", "c", "-I"])
            .arg(&include)
            .arg(include.join("ghostty/vt.h"))
            .output()
            .unwrap();
        assert!(headers.status.success(), "{}", String::from_utf8_lossy(&headers.stderr));
        let from_headers = parse_header_symbols(&String::from_utf8(headers.stdout).unwrap());
        let from_manifest = EXPORTED_API_SYMBOLS.iter().map(|symbol| (*symbol).to_owned()).collect();
        assert_eq!(from_headers, from_manifest);
    }

    #[test]
    fn exported_manifest_matches_bindings() {
        let from_bindings = parse_binding_symbols(include_str!("bindings.rs"));
        let from_manifest: BTreeSet<String> = EXPORTED_API_SYMBOLS
            .iter()
            .map(|symbol| (*symbol).to_owned())
            .collect();
        assert_eq!(from_manifest, from_bindings);
    }

    #[test]
    fn exported_manifest_is_sorted_and_unique() {
        let mut prev = "";
        for symbol in EXPORTED_API_SYMBOLS {
            assert!(
                *symbol > prev,
                "EXPORTED_API_SYMBOLS is not sorted or has duplicates: {prev:?} >= {symbol:?}"
            );
            prev = symbol;
        }
    }
}
