use anyhow::Result;
use futures::{
    channel::{mpsc, oneshot},
    StreamExt,
};
use gpui::{
    actions, anchored, canvas, deferred, div, fill, linear_color_stop, linear_gradient, point,
    prelude::*, px, relative, rgb, Anchor, Animation, AnimationExt, App, Bounds, Context, Element,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, GlobalElementId,
    IntoElement, LayoutId, Menu, MenuItem, PathPromptOptions, Pixels, Point, QuitMode, Render,
    Role, SharedString, Style, Subscription, TextRun, TitlebarOptions, UTF16Selection, WeakEntity,
    Window, WindowAppearance, WindowBounds, WindowOptions, WrappedLine,
};
use gpui_platform::application;
use mew_browser_host::{BrowserEvent, BrowserPortal, BrowserRect, PumpTrigger};
use mew_client_core::{
    ActionKind, ClientEngine, ClientEvent, ClientState, ClientTransport, PendingAction,
};
use mew_client_iroh::IrohTransport;
use mew_client_local::LocalWebSocketTransport;
use mew_desktop_supervisor::{DaemonEndpoint, DaemonMode, DesktopSupervisor, SupervisorConfig};
use mew_diff::{DiffLine, FileDiff, FileStatus};
use mew_protocol::{Attachment, ClientMessage, DirEntry, PermissionDecision};
use mew_tui::theme::Theme;
use mew_ui_model::{
    ActivityTodoStatus, ConversationItem, ToolStatus, TranscriptItem, TranscriptPart,
    TranscriptRole, UsageSummary,
};
use ratatui::style::Color;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    icons::{tabler_icon, IconAssets, TablerIcon},
    markdown::{
        highlight_code_blocks, parse_document, virtualize_document, InlineStyle, InlineText,
        MarkdownBlock, MarkdownRenderBlock, MarkdownSyntaxHighlight, StreamingMarkdown,
    },
    model::ShellModel,
    terminal::{TerminalEvent, TerminalView, DEFAULT_FONT_FAMILY},
};

mod chat;
mod chat_render;
mod composer;
mod helpers;
mod lifecycle;
mod platform;
mod preferences;
mod render;
mod session;
mod session_data;
mod settings;
mod sidebar;
mod tests;
mod topbar;
mod types;
mod workbench;

use composer::{ComposerElement, TextInputTarget};
use helpers::*;
pub(crate) use platform::run;
use types::*;

actions!(
    mew_desktop,
    [
        Quit,
        NewConversation,
        CloseConversation,
        ToggleSidebar,
        ToggleTerminal,
        ToggleWorkbench,
        DismissPopovers,
        ComposerBackspace,
        ComposerDelete,
        ComposerLeft,
        ComposerRight,
        ComposerSelectLeft,
        ComposerSelectRight,
        ComposerSelectAll,
        ComposerHome,
        ComposerEnd,
        ComposerPaste,
    ]
);
