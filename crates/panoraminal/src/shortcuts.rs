//! Keyboard shortcuts: a single table used both for handling and for the Help window.

use eframe::egui::{Key, KeyboardShortcut, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ToolPen,
    ToolBrush,
    ToolAirbrush,
    ToolEraser,
    ToolEyedropper,
    ToolFill,
    SizeDown,
    SizeUp,
    HardnessDown,
    HardnessUp,
    Undo,
    Redo,
    New,
    Open,
    Save,
    SaveAs,
    ExportPng,
    ExportJpg,
    ImportLayer,
    NewLayer,
    DuplicateLayer,
    MergeDown,
    ClearLayer,
    ToggleView,
    ToggleGrid,
    ResetView,
    ZoomIn,
    ZoomOut,
    HideUi,
    Help,
    Fullscreen,
    SwapColors,
    Settings,
    Quit,
}

pub struct Shortcut {
    pub action: Action,
    pub keys: &'static [KeyboardShortcut],
    /// i18n key of the description.
    pub label: &'static str,
    pub group: Group,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Tools,
    Brush,
    File,
    Edit,
    Layers,
    View,
}

impl Group {
    pub const ALL: [Group; 6] = [
        Group::Tools,
        Group::Brush,
        Group::File,
        Group::Edit,
        Group::Layers,
        Group::View,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Group::Tools => "help.group.tools",
            Group::Brush => "help.group.brush",
            Group::File => "help.group.file",
            Group::Edit => "help.group.edit",
            Group::Layers => "help.group.layers",
            Group::View => "help.group.view",
        }
    }
}

const NONE: Modifiers = Modifiers::NONE;
const CMD: Modifiers = Modifiers::COMMAND;
const SHIFT: Modifiers = Modifiers::SHIFT;
const CMD_SHIFT: Modifiers = Modifiers {
    shift: true,
    ..Modifiers::COMMAND
};

const fn k(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(modifiers, key)
}

pub static TABLE: &[Shortcut] = &[
    Shortcut {
        action: Action::ToolPen,
        keys: &[k(NONE, Key::P)],
        label: "tool.pen",
        group: Group::Tools,
    },
    Shortcut {
        action: Action::ToolBrush,
        keys: &[k(NONE, Key::B)],
        label: "tool.brush",
        group: Group::Tools,
    },
    Shortcut {
        action: Action::ToolAirbrush,
        keys: &[k(NONE, Key::A)],
        label: "tool.airbrush",
        group: Group::Tools,
    },
    Shortcut {
        action: Action::ToolEraser,
        keys: &[k(NONE, Key::E)],
        label: "tool.eraser",
        group: Group::Tools,
    },
    Shortcut {
        action: Action::ToolEyedropper,
        keys: &[k(NONE, Key::I)],
        label: "tool.eyedropper",
        group: Group::Tools,
    },
    Shortcut {
        action: Action::ToolFill,
        keys: &[k(NONE, Key::F)],
        label: "tool.fill",
        group: Group::Tools,
    },
    Shortcut {
        action: Action::SwapColors,
        keys: &[k(NONE, Key::X)],
        label: "action.swap_colors",
        group: Group::Tools,
    },
    Shortcut {
        action: Action::HardnessDown,
        keys: &[k(SHIFT, Key::OpenBracket), k(NONE, Key::OpenCurlyBracket)],
        label: "action.hardness_down",
        group: Group::Brush,
    },
    Shortcut {
        action: Action::HardnessUp,
        keys: &[k(SHIFT, Key::CloseBracket), k(NONE, Key::CloseCurlyBracket)],
        label: "action.hardness_up",
        group: Group::Brush,
    },
    Shortcut {
        action: Action::SizeDown,
        keys: &[k(NONE, Key::OpenBracket)],
        label: "action.size_down",
        group: Group::Brush,
    },
    Shortcut {
        action: Action::SizeUp,
        keys: &[k(NONE, Key::CloseBracket)],
        label: "action.size_up",
        group: Group::Brush,
    },
    Shortcut {
        action: Action::New,
        keys: &[k(CMD, Key::N)],
        label: "menu.file.new",
        group: Group::File,
    },
    Shortcut {
        action: Action::Open,
        keys: &[k(CMD, Key::O)],
        label: "menu.file.open",
        group: Group::File,
    },
    Shortcut {
        action: Action::SaveAs,
        keys: &[k(CMD_SHIFT, Key::S)],
        label: "menu.file.save_as",
        group: Group::File,
    },
    Shortcut {
        action: Action::Save,
        keys: &[k(CMD, Key::S)],
        label: "menu.file.save",
        group: Group::File,
    },
    Shortcut {
        action: Action::ExportJpg,
        keys: &[k(CMD_SHIFT, Key::E)],
        label: "menu.file.export_jpg",
        group: Group::File,
    },
    Shortcut {
        action: Action::ExportPng,
        keys: &[k(CMD, Key::E)],
        label: "menu.file.export_png",
        group: Group::File,
    },
    Shortcut {
        action: Action::ImportLayer,
        keys: &[k(CMD, Key::I)],
        label: "menu.file.import",
        group: Group::File,
    },
    Shortcut {
        action: Action::Settings,
        keys: &[k(CMD, Key::Comma)],
        label: "menu.file.settings",
        group: Group::File,
    },
    Shortcut {
        action: Action::Quit,
        keys: &[k(CMD, Key::Q)],
        label: "menu.file.quit",
        group: Group::File,
    },
    Shortcut {
        action: Action::Redo,
        keys: &[k(CMD_SHIFT, Key::Z), k(CMD, Key::Y)],
        label: "menu.edit.redo",
        group: Group::Edit,
    },
    Shortcut {
        action: Action::Undo,
        keys: &[k(CMD, Key::Z)],
        label: "menu.edit.undo",
        group: Group::Edit,
    },
    Shortcut {
        action: Action::ClearLayer,
        keys: &[k(NONE, Key::Delete), k(NONE, Key::Backspace)],
        label: "menu.edit.clear",
        group: Group::Edit,
    },
    Shortcut {
        action: Action::NewLayer,
        keys: &[k(CMD_SHIFT, Key::N)],
        label: "menu.layer.new",
        group: Group::Layers,
    },
    Shortcut {
        action: Action::DuplicateLayer,
        keys: &[k(CMD, Key::J)],
        label: "menu.layer.duplicate",
        group: Group::Layers,
    },
    Shortcut {
        action: Action::MergeDown,
        keys: &[k(CMD, Key::M)],
        label: "menu.layer.merge_down",
        group: Group::Layers,
    },
    Shortcut {
        action: Action::ToggleView,
        keys: &[k(NONE, Key::V)],
        label: "menu.view.toggle",
        group: Group::View,
    },
    Shortcut {
        action: Action::ToggleGrid,
        keys: &[k(NONE, Key::G)],
        label: "menu.view.grid",
        group: Group::View,
    },
    Shortcut {
        action: Action::ResetView,
        keys: &[k(NONE, Key::Num0)],
        label: "menu.view.reset",
        group: Group::View,
    },
    Shortcut {
        action: Action::ZoomIn,
        keys: &[k(NONE, Key::Equals), k(NONE, Key::Plus)],
        label: "menu.view.zoom_in",
        group: Group::View,
    },
    Shortcut {
        action: Action::ZoomOut,
        keys: &[k(NONE, Key::Minus)],
        label: "menu.view.zoom_out",
        group: Group::View,
    },
    Shortcut {
        action: Action::HideUi,
        keys: &[k(NONE, Key::Tab)],
        label: "menu.view.hide_ui",
        group: Group::View,
    },
    Shortcut {
        action: Action::Fullscreen,
        keys: &[k(NONE, Key::F11)],
        label: "menu.view.fullscreen",
        group: Group::View,
    },
    Shortcut {
        action: Action::Help,
        keys: &[k(NONE, Key::F1)],
        label: "menu.help.help",
        group: Group::View,
    },
];

/// Gestures that are not single key presses (shown in the Help window).
pub static GESTURES: &[(&str, &str, Group)] = &[
    ("Space + help.key.drag", "help.gesture.space", Group::View),
    ("help.key.right_drag", "help.gesture.rotate", Group::View),
    ("help.key.wheel", "help.gesture.zoom", Group::View),
    ("help.key.arrows", "help.gesture.arrows", Group::View),
    ("Shift + help.key.click", "help.gesture.line", Group::Brush),
    ("Alt + help.key.click", "help.gesture.pick", Group::Tools),
    ("help.key.pinch", "help.gesture.pinch", Group::View),
    ("Esc", "help.gesture.esc", Group::Edit),
];

/// Human friendly key name: symbols instead of egui's `OpenBracket` style names.
pub fn key_name(k: eframe::egui::Key) -> String {
    use eframe::egui::Key;
    match k {
        Key::OpenBracket => "[".into(),
        Key::CloseBracket => "]".into(),
        Key::OpenCurlyBracket => "{".into(),
        Key::CloseCurlyBracket => "}".into(),
        Key::Equals => "=".into(),
        Key::Plus => "+".into(),
        Key::Minus => "-".into(),
        Key::Comma => ",".into(),
        Key::Period => ".".into(),
        Key::Delete => "Del".into(),
        Key::Escape => "Esc".into(),
        Key::Space => "Space".into(),
        other => other.symbol_or_name().to_string(),
    }
}

/// Compact form for menus and tooltips, e.g. `Ctrl+Shift+S`.
pub fn shortcut_text(ctx: &eframe::egui::Context, sc: &KeyboardShortcut) -> String {
    shortcut_text_sep(ctx, sc, "+")
}

/// Formats a shortcut with the given separator (`⌘` instead of Ctrl on macOS).
pub fn shortcut_text_sep(ctx: &eframe::egui::Context, sc: &KeyboardShortcut, sep: &str) -> String {
    let mac = ctx.os() == eframe::egui::os::OperatingSystem::Mac;
    let m = sc.modifiers;
    let mut parts: Vec<String> = Vec::new();
    if m.command || m.ctrl || m.mac_cmd {
        parts.push(if mac { "⌘".into() } else { "Ctrl".into() });
    }
    if m.alt {
        parts.push(if mac { "Option".into() } else { "Alt".into() });
    }
    if m.shift {
        parts.push("Shift".into());
    }
    parts.push(key_name(sc.logical_key));
    parts.join(sep)
}

pub fn first_text(ctx: &eframe::egui::Context, a: Action) -> String {
    TABLE
        .iter()
        .find(|s| s.action == a)
        .and_then(|s| s.keys.first())
        .map(|k| shortcut_text(ctx, k))
        .unwrap_or_default()
}
