//! Toolbar icons (24×24, stroked; drawn with `currentColor`).

use leptos::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Keyboard,
    Clipboard,
    Zoom,
    Eye,
    EyeOff,
    Camera,
    Refresh,
    Maximize,
    Minimize,
    NewWindow,
    Power,
    Lock,
    Unlock,
    Star,
    Close,
    Chevron,
    Key,
}

impl Icon {
    fn paths(self) -> &'static str {
        match self {
            Icon::Keyboard => {
                r#"<rect x="2" y="5" width="20" height="14" rx="2"/><path d="M6 9h.01M10 9h.01M14 9h.01M18 9h.01M8 13h.01M12 13h.01M16 13h.01M7 16h10"/>"#
            }
            Icon::Clipboard => {
                r#"<rect x="8" y="2" width="8" height="4" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><path d="M12 11v6M9 14l3 3 3-3"/>"#
            }
            Icon::Zoom => r#"<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>"#,
            Icon::Eye => {
                r#"<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>"#
            }
            Icon::EyeOff => {
                r#"<path d="M9.9 4.24A9 9 0 0 1 12 4c6.5 0 10 8 10 8a17 17 0 0 1-2.16 3.19M6.6 6.6A16 16 0 0 0 2 12s3.5 7 10 7a9.7 9.7 0 0 0 5.4-1.6"/><path d="M14.12 14.12a3 3 0 1 1-4.24-4.24"/><path d="m2 2 20 20"/>"#
            }
            Icon::Camera => {
                r#"<path d="M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3z"/><circle cx="12" cy="13" r="3"/>"#
            }
            Icon::Refresh => r#"<path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><path d="M21 3v5h-5"/>"#,
            Icon::Maximize => {
                r#"<path d="M8 3H5a2 2 0 0 0-2 2v3M21 8V5a2 2 0 0 0-2-2h-3M3 16v3a2 2 0 0 0 2 2h3M16 21h3a2 2 0 0 0 2-2v-3"/>"#
            }
            Icon::Minimize => {
                r#"<path d="M8 3v3a2 2 0 0 1-2 2H3M21 8h-3a2 2 0 0 1-2-2V3M3 16h3a2 2 0 0 1 2 2v3M16 21v-3a2 2 0 0 1 2-2h3"/>"#
            }
            Icon::NewWindow => {
                r#"<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18M12 12v5M9.5 14.5h5"/>"#
            }
            Icon::Power => r#"<path d="M12 2v10"/><path d="M18.4 6.6a9 9 0 1 1-12.77.04"/>"#,
            Icon::Lock => {
                r#"<rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>"#
            }
            Icon::Unlock => {
                r#"<rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 7.9-1"/>"#
            }
            Icon::Star => {
                r#"<path d="M12 2.5l2.9 6 6.6.9-4.8 4.6 1.2 6.5L12 17.4l-5.9 3.1 1.2-6.5-4.8-4.6 6.6-.9z"/>"#
            }
            Icon::Close => r#"<path d="M18 6 6 18M6 6l12 12"/>"#,
            Icon::Chevron => r#"<path d="m6 9 6 6 6-6"/>"#,
            Icon::Key => {
                r#"<circle cx="7.5" cy="15.5" r="4.5"/><path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"/>"#
            }
        }
    }
}

#[component]
pub fn Svg(icon: Icon) -> impl IntoView {
    view! { <svg viewBox="0 0 24 24" class="icon" aria-hidden="true" inner_html=icon.paths()></svg> }
}
