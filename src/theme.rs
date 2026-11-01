//! Colours and widget styles taken from gc2's dark theme:
//! #2b2b2b background, #313335 posts, #a9b7c6 text, #4eadf1 clocks/links,
//! #cc7832 authors, #6a8759 totoz and #ff5f5f replies/bigornos.

use iced::widget::{button, container, scrollable, text};
use iced::{Background, Border, Color, Shadow, Theme};

use crate::config::ThemeMode;

pub const ACCENT: Color = Color::from_rgb8(0x4e, 0xad, 0xf1);
pub const ACCENT_DARK: Color = Color::from_rgb8(0x2f, 0x7f, 0xb8);
pub const CLOCK: Color = Color::from_rgb8(0x4e, 0xad, 0xf1);
pub const NAME: Color = Color::from_rgb8(0xcc, 0x78, 0x32);
pub const MUTED: Color = Color::from_rgb8(0xa0, 0xa0, 0xa0);
pub const DANGER: Color = Color::from_rgb8(0xff, 0x5f, 0x5f);
pub const SUCCESS: Color = Color::from_rgb8(0x6a, 0x87, 0x59);
pub const OWN_POST: Color = Color::from_rgb8(0xf4, 0xc5, 0x42);
pub const LINK: Color = Color::from_rgb8(0x4e, 0xad, 0xf1);
pub const TOTOZ: Color = Color::from_rgb8(0x6a, 0x87, 0x59);
pub const BIGORNO: Color = Color::from_rgb8(0xff, 0x5f, 0x5f);

pub const CONVERSATION: [Color; crate::conversation::CONVERSATION_COLORS] = [
    Color::from_rgb(1.0, 0.37, 0.53),
    Color::from_rgb(0.37, 0.68, 1.0),
    Color::from_rgb(0.37, 0.84, 0.53),
    Color::from_rgb(1.0, 0.84, 0.37),
    Color::from_rgb(0.69, 0.53, 1.0),
    Color::from_rgb(1.0, 0.53, 0.37),
    Color::from_rgb(0.37, 1.0, 1.0),
    Color::from_rgb(0.84, 0.69, 0.37),
];

pub fn theme(mode: ThemeMode) -> Theme {
    match mode {
        ThemeMode::Light => Theme::Light,
        ThemeMode::Dark => Theme::Dark,
        ThemeMode::System => {
            let dark = std::env::var("GTK_THEME")
                .map(|value| value.to_lowercase().contains("dark"))
                .unwrap_or(false)
                || std::env::var("COLORFGBG")
                    .ok()
                    .and_then(|value| {
                        value
                            .rsplit(';')
                            .next()
                            .and_then(|part| part.parse::<u8>().ok())
                    })
                    .map(|background| background < 8)
                    .unwrap_or(true);
            if dark {
                Theme::Dark
            } else {
                Theme::Light
            }
        }
    }
}

pub fn background(theme: &Theme) -> Color {
    if theme.extended_palette().is_dark {
        Color::from_rgb8(0x2b, 0x2b, 0x2b)
    } else {
        Color::from_rgb8(0xee, 0xee, 0xee)
    }
}

pub fn panel_color(theme: &Theme) -> Color {
    if theme.extended_palette().is_dark {
        Color::from_rgb8(0x31, 0x33, 0x35)
    } else {
        Color::from_rgb8(0xfb, 0xfb, 0xfb)
    }
}

pub fn card_color(theme: &Theme) -> Color {
    if theme.extended_palette().is_dark {
        Color::from_rgb8(0x31, 0x33, 0x35)
    } else {
        Color::from_rgb8(0xff, 0xff, 0xff)
    }
}

pub fn card_selected(theme: &Theme) -> Color {
    if theme.extended_palette().is_dark {
        Color::from_rgb8(0x3e, 0x3e, 0x3e)
    } else {
        Color::from_rgb8(0xd7, 0xd2, 0xd2)
    }
}

pub fn text_color(theme: &Theme) -> Color {
    if theme.extended_palette().is_dark {
        Color::from_rgb8(0xa9, 0xb7, 0xc6)
    } else {
        Color::from_rgb8(0x00, 0x00, 0x00)
    }
}

pub fn muted_color(theme: &Theme) -> Color {
    if theme.extended_palette().is_dark {
        Color::from_rgb8(0xa0, 0xa0, 0xa0)
    } else {
        Color::from_rgb8(0x77, 0x77, 0x77)
    }
}

pub fn app_background(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(background(theme))),
        text_color: Some(text_color(theme)),
        ..container::Style::default()
    }
}

pub fn sidebar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(panel_color(theme))),
        text_color: Some(text_color(theme)),
        border: Border {
            color: Color { a: 0.12, ..ACCENT },
            width: 0.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn panel(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(card_color(theme))),
        text_color: Some(text_color(theme)),
        border: Border {
            color: Color {
                a: 0.18,
                ..text_color(theme)
            },
            width: 1.0,
            radius: 10.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn card(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(card_color(theme))),
        text_color: Some(text_color(theme)),
        border: Border {
            color: Color {
                a: 0.10,
                ..text_color(theme)
            },
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn post_row(
    theme: &Theme,
    selected: bool,
    hovered: bool,
    hidden: bool,
    index: usize,
) -> container::Style {
    let background = if selected {
        card_selected(theme)
    } else if hovered {
        if theme.extended_palette().is_dark {
            Color::from_rgb8(0x59, 0x4f, 0x1f)
        } else {
            Color::from_rgb8(0xff, 0xd2, 0x02)
        }
    } else if hidden {
        if theme.extended_palette().is_dark {
            Color::from_rgb8(0x50, 0x50, 0x50)
        } else {
            Color::from_rgb8(0xd0, 0xd0, 0xd0)
        }
    } else if index.is_multiple_of(2) {
        if theme.extended_palette().is_dark {
            Color::from_rgb8(0x31, 0x33, 0x35)
        } else {
            Color::from_rgb8(0xff, 0xff, 0xff)
        }
    } else if theme.extended_palette().is_dark {
        Color::from_rgb8(0x2b, 0x2b, 0x2b)
    } else {
        Color::from_rgb8(0xfb, 0xfb, 0xfb)
    };
    container::Style {
        background: Some(Background::Color(background)),
        text_color: Some(if hidden {
            muted_color(theme)
        } else {
            text_color(theme)
        }),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 3.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn accent_badge(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(ACCENT)),
        text_color: Some(Color::WHITE),
        border: Border {
            color: ACCENT,
            width: 0.0,
            radius: 999.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn button_primary(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color::from_rgb8(0x6f, 0xc3, 0xff),
        button::Status::Pressed => ACCENT_DARK,
        button::Status::Disabled => Color { a: 0.30, ..ACCENT },
        _ => ACCENT,
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::from_rgb8(0x2b, 0x2b, 0x2b),
        border: Border {
            color: background,
            width: 0.0,
            radius: 7.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn button_secondary(theme: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(Background::Color(if hovered {
            Color { a: 0.20, ..ACCENT }
        } else {
            Color {
                a: 0.08,
                ..text_color(theme)
            }
        })),
        text_color: text_color(theme),
        border: Border {
            color: Color {
                a: 0.20,
                ..text_color(theme)
            },
            width: 1.0,
            radius: 7.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn button_ghost(theme: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then_some(Background::Color(Color {
            a: 0.10,
            ..text_color(theme)
        })),
        text_color: if hovered {
            text_color(theme)
        } else {
            muted_color(theme)
        },
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 6.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn tribune_button(theme: &Theme, status: button::Status, active: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(Background::Color(if active || hovered {
            card_selected(theme)
        } else {
            Color::TRANSPARENT
        })),
        text_color: text_color(theme),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 7.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// Panel holding the inline totoz autocompletion above the composer.
pub fn totoz_suggestion_panel(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(card_color(theme))),
        text_color: Some(text_color(theme)),
        border: Border {
            color: Color { a: 0.30, ..ACCENT },
            width: 1.0,
            radius: 8.0.into(),
        },
        shadow: Shadow {
            color: Color {
                a: 0.35,
                ..Color::BLACK
            },
            offset: iced::Vector::new(0.0, -2.0),
            blur_radius: 8.0,
        },
        snap: true,
    }
}

/// One row of the totoz autocompletion. The active row (keyboard or hover) is
/// shaded so users always know which suggestion Enter will pick.
pub fn totoz_suggestion(theme: &Theme, status: button::Status, active: bool) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: (active || hovered).then_some(Background::Color(Color { a: 0.18, ..ACCENT })),
        text_color: text_color(theme),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 6.0.into(),
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

pub fn text_input_style(
    theme: &Theme,
    status: iced::widget::text_input::Status,
) -> iced::widget::text_input::Style {
    let border = match status {
        iced::widget::text_input::Status::Focused { .. } => ACCENT,
        _ => Color {
            a: 0.20,
            ..text_color(theme)
        },
    };
    iced::widget::text_input::Style {
        background: Background::Color(panel_color(theme)),
        border: Border {
            color: border,
            width: 1.0,
            radius: 7.0.into(),
        },
        icon: muted_color(theme),
        placeholder: muted_color(theme),
        value: text_color(theme),
        selection: Color { a: 0.35, ..ACCENT },
    }
}

pub fn scrollable_style(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    scrollable::default(theme, status)
}

pub fn muted_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(muted_color(theme)),
    }
}

pub fn danger_text(_theme: &Theme) -> text::Style {
    text::Style {
        color: Some(DANGER),
    }
}

pub fn success_text(_theme: &Theme) -> text::Style {
    text::Style {
        color: Some(SUCCESS),
    }
}

pub fn rule(theme: &Theme) -> iced::widget::rule::Style {
    iced::widget::rule::Style {
        color: Color {
            a: 0.12,
            ..text_color(theme)
        },
        radius: 0.0.into(),
        fill_mode: iced::widget::rule::FillMode::Full,
        snap: true,
    }
}
