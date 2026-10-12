use mew_tui::{
    app::{Mode, PickerItem},
    App,
};

pub fn run(gallery: &mut super::Gallery) {
    let mut app = App::new();
    app.push_synthetic_message("the session fixes are verified. next i’ll check the picker layout and keep the selected choice visible while resizing.".into());
    app.input = "/".into();
    app.cursor = 1;
    app.mode = Mode::SlashCommand;
    gallery.capture(&mut app, "desktop-inline-commands", 128, 40);
    app.slash_selected = app.filtered_slash_commands().len() - 1;
    gallery.capture(&mut app, "desktop-inline-commands-end", 128, 40);
    gallery.capture(&mut app, "short-inline-commands", 40, 16);
    let mut app = App::new();
    app.input = "/".into();
    app.cursor = 1;
    app.mode = Mode::SlashCommand;
    gallery.capture(&mut app, "desktop-landing-commands", 128, 40);
    let mut app = App::new();
    app.open_command_palette();
    gallery.capture(&mut app, "desktop-command-palette", 128, 40);
    let mut app = App::new();
    app.push_synthetic_message("choose a project file to include in the next instruction".into());
    app.input = "@".into();
    app.cursor = 1;
    app.open_at_picker();
    app.picker.as_mut().unwrap().items = (0..60)
        .map(|i| PickerItem {
            id: format!("src/file{i:02}.rs"),
            label: format!("@src/file{i:02}.rs"),
            description: "project source file".into(),
            ..Default::default()
        })
        .collect();
    gallery.capture(&mut app, "desktop-inline-references", 128, 40);
    app.picker.as_mut().unwrap().items.truncate(2);
    gallery.capture(&mut app, "desktop-inline-references-short", 128, 40);
}
