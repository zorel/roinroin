//! Widget tree for roinroin.

use std::collections::BTreeMap;

use iced::widget::text::Span;
use iced::widget::{
    button, checkbox, column, container, image, mouse_area, row, rule, scrollable, span, text,
    text_input, Space,
};
use iced::{mouse, Alignment, Color, Element, Fill, Font, Length, Vector};

use crate::app::{
    App, ConnectionStatus, LinkAction, LinkPreview, Message, Screen, SettingsMessage,
};
use crate::config::{BlockedMode, BoardAuth, ThemeMode};
use crate::hover_text::HoverRich;
use crate::message::Token;
use crate::model::{PostKey, SourceId, TribuneKey};
use crate::theme as colors;

pub fn view(app: &App) -> Element<'_, Message> {
    let content = match app.screen {
        Screen::Main => main_view(app),
        Screen::Settings => settings_view(app),
        Screen::Totoz => totoz_view(app),
        Screen::Emoji => emoji_view(app),
    };
    let base: Element<'_, Message> = container(content)
        .style(colors::app_background)
        .width(Fill)
        .height(Fill)
        .into();
    // Always keep the same stack structure. Adding/removing the overlay changed
    // the widget tree root and reset the scroll position when a totoz was
    // hovered.
    let preview = if app.screen == Screen::Main {
        post_preview_overlay(app)
    } else {
        Space::new().width(Fill).height(Fill).into()
    };
    let totoz = if app.screen == Screen::Main {
        totoz_hover_overlay(app)
    } else {
        Space::new().width(Fill).height(Fill).into()
    };
    let link = if app.screen == Screen::Main {
        link_preview_overlay(app)
    } else {
        Space::new().width(Fill).height(Fill).into()
    };
    iced::widget::stack![base, preview, totoz, link].into()
}

fn post_preview_overlay(app: &App) -> Element<'_, Message> {
    let Some(key) = &app.hovered_preview else {
        return Space::new().width(Fill).height(Fill).into();
    };
    if !app.posts.contains_key(key) {
        return Space::new().width(Fill).height(Fill).into();
    }

    let index = app
        .visible_keys()
        .iter()
        .position(|item| item == key)
        .unwrap_or(0);
    let content = post_card_body(app, key.clone(), index, false);
    let window_width = app.window_size.width.max(1.0);
    let window_height = app.window_size.height.max(1.0);
    let content_left = app.sidebar_width + 6.0;
    let content_width = (window_width - content_left).max(1.0);
    let width = (content_width - 64.0).clamp(320.0, 760.0);
    let left = content_left + (content_width - width) / 2.0;
    let top = 8.0;
    let max_height = (window_height - top - 16.0).max(120.0);

    let panel = mouse_area(
        container(content)
            .width(Length::Fixed(width))
            .max_height(max_height)
            .clip(true)
            .style(colors::panel)
            .padding(6),
    )
    .on_enter(Message::PreviewEnter)
    .on_exit(Message::PreviewExit);

    iced::widget::float(panel)
        .translate(move |bounds, _viewport| Vector::new(left - bounds.x, top - bounds.y))
        .into()
}

fn totoz_hover_overlay(app: &App) -> Element<'_, Message> {
    let Some(totoz) = &app.hovered_totoz else {
        return Space::new().width(Fill).height(Fill).into();
    };
    let Some(preview) = &totoz.image else {
        return Space::new().width(Fill).height(Fill).into();
    };
    let width = preview.width as f32;
    let height = preview.height as f32;
    let anchor = totoz.anchor;
    let window_width = app.window_size.width.max(1.0);
    let window_height = app.window_size.height.max(1.0);

    let frame = preview
        .frames
        .get(totoz.frame_index)
        .unwrap_or_else(|| preview.first_frame());
    iced::widget::float(image(frame.clone()).width(width).height(height))
        .translate(move |bounds, _viewport| {
            let left = (anchor.x - bounds.width / 2.0)
                .clamp(4.0, (window_width - bounds.width - 4.0).max(4.0));
            let above = anchor.y - bounds.height - 5.0;
            let top = if above >= 4.0 {
                above
            } else {
                (anchor.y + 18.0).min((window_height - bounds.height - 4.0).max(4.0))
            };
            Vector::new(left - bounds.x, top - bounds.y)
        })
        .into()
}

fn link_preview_overlay(app: &App) -> Element<'_, Message> {
    let Some(hovered) = &app.hovered_link_preview else {
        return Space::new().width(Fill).height(Fill).into();
    };
    let Some(preview) = app.link_previews.get(&hovered.url) else {
        return Space::new().width(Fill).height(Fill).into();
    };

    let content: Element<'_, Message> = match preview {
        LinkPreview::Loading => {
            return Space::new().width(Fill).height(Fill).into();
        }
        LinkPreview::Error => {
            container(text("✗").size(16).color(Color::from_rgb8(0xff, 0x55, 0x55)))
                .padding([1, 4])
                .style(colors::panel)
                .into()
        }
        LinkPreview::Title(title) => container(text(title.as_str()).size(13))
            .padding(6)
            .max_width(420)
            .style(colors::panel)
            .into(),
        LinkPreview::Image {
            handle,
            width,
            height,
        } => image(handle.clone())
            .width(*width as f32)
            .height(*height as f32)
            .into(),
    };

    let anchor = hovered.anchor;
    let window_width = app.window_size.width.max(1.0);
    let window_height = app.window_size.height.max(1.0);

    iced::widget::float(content)
        .translate(move |bounds, _viewport| {
            let left = (anchor.x - bounds.width / 2.0)
                .clamp(4.0, (window_width - bounds.width - 4.0).max(4.0));
            let above = anchor.y - bounds.height - 5.0;
            let top = if above >= 4.0 {
                above
            } else {
                (anchor.y + 5.0).min((window_height - bounds.height - 4.0).max(4.0))
            };
            Vector::new(left - bounds.x, top - bounds.y)
        })
        .into()
}

fn main_view(app: &App) -> Element<'_, Message> {
    let divider = mouse_area(
        container(rule::vertical(1).style(colors::rule))
            .width(6)
            .height(Fill)
            .align_x(Alignment::Center),
    )
    .on_press(Message::SidebarDragStart)
    .interaction(mouse::Interaction::ResizingHorizontally);

    let body = row![
        container(sidebar(app))
            .width(Length::Fixed(app.sidebar_width))
            .height(Fill)
            .style(colors::sidebar),
        divider,
        container(content_view(app))
            .width(Fill)
            .height(Fill)
            .padding(12),
    ]
    .height(Fill);
    body.into()
}

fn sidebar(app: &App) -> Element<'_, Message> {
    let mut gb2c_groups: BTreeMap<String, (String, Vec<TribuneKey>)> = BTreeMap::new();
    let mut direct = Vec::new();
    for tribune in app.tribunes() {
        match &tribune.source {
            SourceId::Gb2c(url) => {
                let label = format!("gb2c · {}", app.source_label(&SourceId::Gb2c(url.clone())));
                gb2c_groups
                    .entry(label)
                    .or_insert_with(|| (url.clone(), Vec::new()))
                    .1
                    .push(tribune);
            }
            SourceId::Direct(_) => direct.push(tribune),
        }
    }
    let mut groups: Vec<(String, Option<String>, Vec<TribuneKey>)> = gb2c_groups
        .into_iter()
        .map(|(label, (url, tribunes))| (label, Some(url), tribunes))
        .collect();
    if !direct.is_empty() {
        groups.push(("boards directs".to_owned(), None, direct));
    }

    let mut list = column![].spacing(2).padding([10, 8]);
    if groups.is_empty() {
        list = list.push(
            container(
                column![
                    text("Aucune tribune").style(colors::muted_text),
                    text("Ouvre les réglages pour ajouter un serveur gb2c ou activer un board direct.")
                        .size(12)
                        .style(colors::muted_text),
                ]
                .spacing(6),
            )
            .padding(8),
        );
    }
    for (group, server_url, tribunes) in groups {
        let mut header = row![text(group.to_uppercase())
            .size(10)
            .style(colors::muted_text)
            .width(Fill)]
        .align_y(Alignment::Center)
        .width(Fill);
        if let Some(status) = server_url.as_deref().map(|url| app.server_status(url)) {
            header = header.push(status_icon(status));
        }
        list = list.push(container(header).padding([0, 8]).width(Fill));
        for tribune in tribunes {
            let active = app.is_active(&tribune);
            let (unread, bigorno, reply) = app.badges(&tribune);
            let mut badges = row![].spacing(4).align_y(Alignment::Center);
            if unread > 0 {
                badges = badges.push(
                    container(text(unread.to_string()).size(10))
                        .style(colors::accent_badge)
                        .padding([1, 6]),
                );
            }
            if bigorno {
                badges = badges.push(text("📢").size(12));
            }
            if reply {
                badges = badges.push(text("↩").size(12));
            }
            let mut right = row![badges].align_y(Alignment::Center);
            if let SourceId::Direct(name) = &tribune.source {
                right = right.push(status_icon(app.board_status(name)));
            }
            let label = row![
                text(tribune.name.clone())
                    .size(14)
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::Word),
                right,
            ]
            .align_y(Alignment::Center)
            .width(Fill);
            list = list.push(
                button(label)
                    .on_press(Message::SelectTribune(tribune.clone()))
                    .style(move |theme, status| colors::tribune_button(theme, status, active))
                    .padding([7, 8])
                    .width(Fill),
            );
        }
        list = list.push(Space::new().height(6));
    }

    scrollable(list).height(Fill).into()
}

fn status_icon<'a>(status: ConnectionStatus) -> Element<'a, Message> {
    let (glyph, color) = match status {
        ConnectionStatus::Online => ("●", colors::SUCCESS),
        ConnectionStatus::Error => ("✕", colors::DANGER),
        ConnectionStatus::Working => ("●", colors::ACCENT),
    };
    container(
        text(glyph)
            .size(11)
            .font(Font {
                weight: iced::font::Weight::Bold,
                ..Font::DEFAULT
            })
            .color(color),
    )
    .width(Length::Fixed(12.0))
    .align_x(Alignment::Center)
    .into()
}

fn content_view(app: &App) -> Element<'_, Message> {
    if app.active_tribune().is_none() {
        return welcome_view(app);
    }

    let keys = app.visible_keys();
    let mut list = column![].spacing(0).width(Fill);
    if keys.is_empty() {
        list = list.push(
            container(
                column![
                    text("Rien à afficher").size(18).style(colors::muted_text),
                    text("Les posts arriveront ici dès que la tribune répondra.")
                        .size(13)
                        .style(colors::muted_text),
                ]
                .spacing(6),
            )
            .padding(20)
            .width(Fill),
        );
    } else {
        for (index, key) in keys.iter().enumerate() {
            list = list.push(post_card(app, key.clone(), index));
        }
    }

    let posts = scrollable(container(list).padding([2, 4]))
        .id(app.posts_scroll_id.clone())
        .height(Fill)
        .on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y))
        .style(colors::scrollable_style);

    column![posts, composer(app)].spacing(8).height(Fill).into()
}

fn welcome_view(app: &App) -> Element<'_, Message> {
    container(
        column![
            text("Bienvenue sur roinroin").size(30).font(Font {
                weight: iced::font::Weight::Bold,
                ..Font::DEFAULT
            }),
            text(
                "Un client natif pour les tribunes du standard bouchot. \
                 Il peut lire un serveur gb2c et des boards directs en même temps."
            )
            .size(15)
            .style(colors::muted_text),
            Space::new().height(12),
            text("Pour commencer :").size(16),
            text("1. Vérifie l'URL de ton serveur gb2c (Réglages).").style(colors::muted_text),
            text("2. Active un ou plusieurs boards directs dans Réglages.")
                .style(colors::muted_text),
            text("3. Les notifications de bigorno et de réponses sont activées par défaut.")
                .style(colors::muted_text),
            Space::new().height(12),
            row![
                button("Ouvrir les réglages")
                    .on_press(Message::OpenSettings)
                    .style(colors::button_primary),
                button("Actualiser")
                    .on_press(Message::RefreshAll)
                    .style(colors::button_secondary),
            ]
            .spacing(8),
            Space::new().height(12),
            text(format!(
                "Serveur : {}",
                app.config
                    .servers
                    .first()
                    .map(|server| server.url.as_str())
                    .unwrap_or("aucun")
            ))
            .size(12)
            .style(colors::muted_text),
        ]
        .spacing(6)
        .max_width(720),
    )
    .padding(40)
    .width(Fill)
    .height(Fill)
    .into()
}

const POST_FONT_SIZE: f32 = 14.0;
const CLOCK_WIDTH: f32 = 78.0;
const STATUS_WIDTH: f32 = 18.0;
const IDENTITY_WIDTH: f32 = 120.0;

fn post_card<'a>(app: &'a App, key: PostKey, index: usize) -> Element<'a, Message> {
    let content = post_card_body(app, key.clone(), index, true);
    let visible_key = key.key();
    crate::visible::Visible::new(
        content,
        visible_key,
        Message::PostShown(key.clone()),
        Message::PostHidden(key),
    )
    .into()
}

fn post_card_body<'a>(
    app: &'a App,
    key: PostKey,
    index: usize,
    interactive: bool,
) -> Element<'a, Message> {
    let Some(post) = app.posts.get(&key) else {
        return Space::new().into();
    };
    let Some(info) = app.analysis.info(&key) else {
        return Space::new().into();
    };
    let parsed = app.parsed_display(&key);
    let selected = app.selected.as_ref() == Some(&key);
    let hovered =
        app.hovered_post.as_ref() == Some(&key) || app.hovered_link_post.as_ref() == Some(&key);
    let hidden = info.hidden;
    let conversation_color = info
        .color
        .map(|index| colors::CONVERSATION[index % colors::CONVERSATION.len()])
        .unwrap_or(colors::MUTED);

    // The timestamp is its own hover target: hovering it highlights the post
    // and every norloge that points at it. Clicking it still quotes the post.
    let clock_color = if selected || hovered {
        Color::WHITE
    } else if hidden {
        colors::MUTED
    } else {
        colors::CLOCK
    };
    let clock = container(
        text(post.time().clock())
            .size(POST_FONT_SIZE)
            .font(Font {
                family: iced::font::Family::Monospace,
                weight: iced::font::Weight::Bold,
                ..Font::DEFAULT
            })
            .color(clock_color),
    )
    .width(Length::Fixed(CLOCK_WIDTH))
    .align_x(Alignment::Start);

    let (status, status_color) = if hidden {
        ("🚫", colors::DANGER)
    } else if info.is_mine {
        ("⭐", colors::SUCCESS)
    } else if info.is_reply {
        ("↩", colors::ACCENT)
    } else if info.is_bigorno {
        ("📢", colors::BIGORNO)
    } else if info.has_duck {
        ("🦆", colors::ACCENT)
    } else {
        ("", colors::MUTED)
    };
    let status_slot: Element<'_, Message> = if status.is_empty() {
        Space::new().width(STATUS_WIDTH).into()
    } else {
        container(text(status).size(POST_FONT_SIZE).color(status_color))
            .width(STATUS_WIDTH)
            .align_x(Alignment::Center)
            .into()
    };

    // Fixed clock, status and identity columns: the message always starts at
    // the same x position, whatever the nickname or UA length.
    let identity_name = crate::model::trim_identity(&post.display_identity(), 16);
    let mut identity_span = span(identity_name)
        .size(POST_FONT_SIZE)
        .color(if hidden { colors::MUTED } else { colors::NAME })
        .font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::DEFAULT
        });
    if !post.identity().is_empty() {
        identity_span = identity_span.link(LinkAction::Info(post.info.clone(), key.clone()));
    }
    let mut identity = HoverRich::with_spans(vec![identity_span])
        .size(POST_FONT_SIZE)
        .width(Length::Fixed(IDENTITY_WIDTH))
        .wrapping(iced::widget::text::Wrapping::None);
    if interactive {
        identity = identity
            .on_link_click(Message::LinkClicked)
            .on_link_hover(Message::HoverLink);
    }

    let mut spans: Vec<Span<'a, LinkAction>> = Vec::new();
    if hidden {
        spans.push(
            span("[message masqué]")
                .size(POST_FONT_SIZE)
                .color(colors::MUTED)
                .font(Font {
                    style: iced::font::Style::Italic,
                    ..Font::DEFAULT
                }),
        );
    } else if let Some(parsed) = parsed {
        spans.extend(message_spans(app, &key, parsed));
    }

    let mut message = HoverRich::with_spans(spans)
        .size(POST_FONT_SIZE)
        .width(Fill);
    if interactive {
        message = message
            .on_link_click(Message::LinkClicked)
            .on_link_hover(Message::HoverLink);
    }

    let rail = container(Space::new())
        .width(3)
        .height(Fill)
        .style(move |_| iced::widget::container::Style {
            background: Some(iced::Background::Color(conversation_color)),
            border: iced::Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 1.0.into(),
            },
            ..iced::widget::container::Style::default()
        });

    let body = row![clock, status_slot, identity, message]
        .spacing(4)
        .align_y(Alignment::Start)
        .width(Fill);
    let card = container(
        row![rail, container(body).padding([3, 7]).width(Fill),]
            .width(Fill)
            .height(Length::Shrink),
    )
    .width(Fill)
    .style(move |theme| colors::post_row(theme, selected, hovered, hidden, index));

    if interactive {
        let hover_key = key.clone();
        iced::widget::mouse_area(card)
            .on_move(move |point| {
                if point.x <= 90.0 {
                    Message::HoverPost(Some(hover_key.clone()))
                } else {
                    Message::HoverPost(None)
                }
            })
            .on_exit(Message::HoverExit)
            .on_press(Message::PostClicked(key))
            .into()
    } else {
        card.into()
    }
}

fn message_spans<'a>(
    app: &'a App,
    key: &PostKey,
    parsed: &'a crate::message::ParsedMessage,
) -> Vec<Span<'a, LinkAction>> {
    let mut spans = Vec::new();
    let info = app.analysis.info(key);
    let highlighted_target = app
        .hovered_post
        .as_ref()
        .or(app.hovered_link_post.as_ref())
        .or(app.selected.as_ref());
    for token in &parsed.tokens {
        match token {
            Token::Text {
                text,
                style,
                spoiler,
            } => {
                let hidden = spoiler.is_some_and(|spoiler| !app.is_revealed(key, spoiler));
                let value = if hidden {
                    "██████"
                } else {
                    text.as_str()
                };
                let mut span = span(value).size(POST_FONT_SIZE);
                if style.bold {
                    span = span.font(Font {
                        weight: iced::font::Weight::Bold,
                        ..Font::DEFAULT
                    });
                }
                if style.monospace {
                    span = span.font(Font::MONOSPACE);
                }
                if style.underline {
                    span = span.underline(true);
                }
                if style.strike {
                    span = span.strikethrough(true);
                }
                if hidden {
                    span = span
                        .color(colors::MUTED)
                        .background(Color::from_rgb8(0x3e, 0x3e, 0x3e));
                    if let Some(spoiler) = spoiler {
                        span = span.link(LinkAction::Spoiler(key.clone(), *spoiler));
                    }
                } else if style.italic && !style.bold {
                    span = span.font(Font {
                        style: iced::font::Style::Italic,
                        ..Font::DEFAULT
                    });
                }
                spans.push(span);
            }
            Token::Link { href, label } => {
                spans.push(
                    span(label.as_str())
                        .size(POST_FONT_SIZE)
                        .color(colors::LINK)
                        .underline(true)
                        .link(LinkAction::Url(href.clone())),
                );
            }
            Token::Totoz { name } => {
                spans.push(
                    span(name.as_str())
                        .size(POST_FONT_SIZE)
                        .color(colors::TOTOZ)
                        .font(Font {
                            weight: iced::font::Weight::Bold,
                            ..Font::DEFAULT
                        })
                        .link(LinkAction::Totoz(name.clone())),
                );
            }
            Token::Norloge {
                reference_index,
                reference,
            } => {
                let target = info
                    .and_then(|info| info.refs.get(*reference_index))
                    .and_then(|resolved| resolved.target.clone());
                let highlighted = target
                    .as_ref()
                    .is_some_and(|target| highlighted_target == Some(target));
                let mut value = span(reference.raw.as_str()).size(POST_FONT_SIZE);
                // Keep the same font weight in both states: a bold span is
                // wider and would shift the rest of the message on hover.
                value = if highlighted {
                    value.color(Color::WHITE).background(colors::ACCENT)
                } else {
                    value.color(colors::CLOCK)
                };
                value = if let Some(target) = target {
                    value.link(LinkAction::Jump(target))
                } else {
                    value.link(LinkAction::Unresolved(reference.raw.clone()))
                };
                spans.push(value);
            }
            Token::Bigorno { word } => {
                spans.push(
                    span(format!("{word}<"))
                        .size(POST_FONT_SIZE)
                        .color(colors::BIGORNO)
                        .background(Color {
                            a: 0.18,
                            ..colors::BIGORNO
                        })
                        .font(Font {
                            weight: iced::font::Weight::Bold,
                            ..Font::DEFAULT
                        }),
                );
            }
        }
    }
    spans
}

fn composer(app: &App) -> Element<'_, Message> {
    let hint = match app.active_tribune() {
        Some(active) => {
            let origin = match &active.source {
                SourceId::Gb2c(url) => {
                    format!("gb2c · {}", app.source_label(&SourceId::Gb2c(url.clone())))
                }
                SourceId::Direct(_) => "board direct".to_owned(),
            };
            format!("{} · {origin}", active.name)
        }
        None => "écrire un message…".to_owned(),
    };

    let editor = text_input(&hint, &app.composer)
        .id(app.composer_id.clone())
        .on_input(Message::Composer)
        .on_submit(Message::ComposerSubmit)
        .size(14)
        .padding([8, 10])
        .width(Fill)
        .style(colors::text_input_style);

    let send = button(container(text("Envoyer").size(14)).center_y(Fill))
        .on_press(Message::Send)
        .style(colors::button_primary)
        .height(Length::Fixed(36.0))
        .padding([0, 16]);

    let toolbar = row![
        Space::new().width(Fill),
        icon_button("🔍", Message::OpenTotoz),
        icon_button("⚙", Message::OpenSettings),
    ]
    .spacing(4)
    .align_y(Alignment::Center)
    .width(Fill);

    // Keep the suggestion slot in the column even when it is empty: the
    // widget tree is reconciled by child position, so inserting a widget
    // above the entry field would reset the text input state (caret and
    // focus) every time the autocompletion appears. The placeholder must not
    // have a fixed zero size, because `Column::push` silently drops void
    // children (a fixed 0 length means "void") and the slot would disappear.
    let suggestions: Element<'_, Message> =
        totoz_suggestions(app).unwrap_or_else(|| Space::new().height(Length::Shrink).into());
    let content = column![
        toolbar,
        suggestions,
        row![editor, send]
            .spacing(6)
            .align_y(Alignment::Center)
            .width(Fill),
    ]
    .spacing(4);

    container(content)
        .style(colors::panel)
        .padding(6)
        .width(Fill)
        .into()
}

/// Inline totoz autocompletion, attached right above the entry field. Nothing
/// is rendered while the list is empty, so an unknown prefix leaves no trace.
fn totoz_suggestions(app: &App) -> Option<Element<'_, Message>> {
    if !app.totoz_suggest.visible() {
        return None;
    }
    let query = app.totoz_suggest.query.to_lowercase();
    let mut list = column![].spacing(1).width(Fill);
    for (index, totoz) in app.totoz_suggest.results.iter().enumerate() {
        let active = app.totoz_suggest.selected == Some(index);
        let name = totoz.name.clone();
        // Keep a fixed box for the thumbnail so rows do not jump while the
        // image is being fetched. The image itself is scaled down to fit.
        let preview: Element<'_, Message> = match app.totoz.images.get(&totoz.name) {
            Some(thumbnail) => image(thumbnail.first_frame().clone())
                .width(Length::Fixed(24.0))
                .height(Length::Fixed(24.0))
                .content_fit(iced::ContentFit::Contain)
                .into(),
            None => Space::new().width(24).height(24).into(),
        };
        let label = totoz_suggestion_label(&totoz.name, &query);
        list = list.push(
            button(
                row![preview, label]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .width(Fill),
            )
            .on_press(Message::TotozSuggestPick(name))
            .style(move |theme, status| colors::totoz_suggestion(theme, status, active))
            .width(Fill)
            .padding([5, 8]),
        );
    }
    Some(
        container(list)
            .style(colors::totoz_suggestion_panel)
            .padding(4)
            .width(Fill)
            .into(),
    )
}

/// Render `[:name]` with the already typed part in normal weight and the
/// predicted remainder in bold, as recommended for autocomplete suggestions.
fn totoz_suggestion_label<'a>(name: &str, query: &str) -> Element<'a, Message> {
    let lower = name.to_lowercase();
    let split = lower
        .find(query)
        .map(|start| start + query.len())
        .filter(|split| name.is_char_boundary(*split))
        .unwrap_or(0);
    let (typed, predicted) = name.split_at(split);
    let font = Font {
        weight: iced::font::Weight::Bold,
        ..Font::DEFAULT
    };
    let mut spans = vec![span(format!("[:{typed}")).size(13).color(colors::TOTOZ)];
    if predicted.is_empty() {
        spans.push(span("]").size(13).color(colors::TOTOZ));
    } else {
        spans.push(
            span(format!("{predicted}]"))
                .size(13)
                .color(colors::TOTOZ)
                .font(font),
        );
    }
    iced::widget::text::Rich::<'_, (), Message, _>::with_spans(spans).into()
}

fn icon_button<'a>(glyph: &'a str, message: Message) -> Element<'a, Message> {
    button(
        container(text(glyph).size(16).style(colors::muted_text))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill),
    )
    .on_press(message)
    .style(colors::button_ghost)
    .width(Length::Fixed(30.0))
    .height(Length::Fixed(28.0))
    .padding(0)
    .into()
}

// ---------------------------------------------------------------------------
// Settings

fn settings_view(app: &App) -> Element<'_, Message> {
    let draft = &app.settings;
    let all_boards = draft_boards(draft);

    let mut servers = column![].spacing(6);
    for (index, server) in draft.servers.iter().enumerate() {
        servers = servers.push(
            row![
                checkbox(server.enabled)
                    .on_toggle(move |_| { Message::Settings(SettingsMessage::ToggleServer(index)) })
                    .size(16),
                text_input("nom", &server.name)
                    .on_input(move |value| Message::Settings(SettingsMessage::ServerName(
                        index, value
                    )))
                    .width(Length::Fixed(140.0))
                    .style(colors::text_input_style),
                text_input("https://gb2c.example", &server.url)
                    .on_input(move |value| Message::Settings(SettingsMessage::ServerUrl(
                        index, value
                    )))
                    .width(Fill)
                    .style(colors::text_input_style),
                button("Supprimer")
                    .on_press(Message::Settings(SettingsMessage::RemoveServer(index)))
                    .style(colors::button_ghost),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .width(Fill),
        );
    }
    servers = servers.push(
        row![
            text_input("ajouter un serveur gb2c", &draft.server_input)
                .on_input(|value| Message::Settings(SettingsMessage::ServerUrlInput(value)))
                .on_submit(Message::Settings(SettingsMessage::AddServer))
                .width(Fill)
                .style(colors::text_input_style),
            button("Ajouter")
                .on_press(Message::Settings(SettingsMessage::AddServer))
                .style(colors::button_secondary),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .width(Fill),
    );

    let mut board_list = column![].spacing(4);
    for board in &all_boards {
        let is_custom = draft
            .custom_boards
            .iter()
            .any(|custom| custom.name == board.name);
        let selected = draft.selected_boards.contains(&board.name);
        let name = board.name.clone();
        let mut row = row![
            checkbox(selected)
                .on_toggle(move |_| {
                    Message::Settings(SettingsMessage::ToggleBoard(name.clone()))
                })
                .size(16),
            text(board.name.clone())
                .size(14)
                .width(Length::Fixed(130.0)),
            text(board.backend_url.clone())
                .size(10)
                .style(colors::muted_text)
                .width(Fill),
            text(match board.auth {
                BoardAuth::Oauth2 => "OAuth2",
                BoardAuth::None => "",
            })
            .size(10)
            .style(colors::muted_text),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .width(Fill);
        if is_custom {
            let name = board.name.clone();
            row = row.push(
                button("Supprimer")
                    .on_press(Message::Settings(SettingsMessage::RemoveBoard(name)))
                    .style(colors::button_ghost),
            );
        }
        board_list = board_list.push(row);
    }

    let add_board = column![
        text("Ajouter un board direct").size(14),
        row![
            text_input("nom", &draft.board_name)
                .on_input(|value| Message::Settings(SettingsMessage::BoardName(value)))
                .width(Length::Fixed(140.0))
                .style(colors::text_input_style),
            text_input("backend TSV/XML", &draft.board_backend)
                .on_input(|value| Message::Settings(SettingsMessage::BoardBackend(value)))
                .width(Fill)
                .style(colors::text_input_style),
        ]
        .spacing(6),
        row![
            text_input("URL de post", &draft.board_post)
                .on_input(|value| Message::Settings(SettingsMessage::BoardPost(value)))
                .width(Fill)
                .style(colors::text_input_style),
            text_input("champ message", &draft.board_post_field)
                .on_input(|value| Message::Settings(SettingsMessage::BoardPostField(value)))
                .width(Length::Fixed(140.0))
                .style(colors::text_input_style),
        ]
        .spacing(6),
        row![
            text_input("champ info (optionnel)", &draft.board_info_field)
                .on_input(|value| Message::Settings(SettingsMessage::BoardInfoField(value)))
                .width(Fill)
                .style(colors::text_input_style),
            checkbox(draft.board_oauth)
                .label("OAuth2 (LinuxFR)")
                .on_toggle(|value| Message::Settings(SettingsMessage::BoardOauth(value)))
                .size(16),
            button("Ajouter le board")
                .on_press(Message::Settings(SettingsMessage::AddBoard))
                .style(colors::button_secondary),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    ]
    .spacing(6);

    let blocked_mode = row![
        button(if draft.blocked_mode == BlockedMode::Gray {
            "Filtrés en gris"
        } else {
            "Griser les filtrés"
        })
        .on_press(Message::Settings(SettingsMessage::BlockedMode(
            BlockedMode::Gray
        )))
        .style(colors::button_secondary),
        button(if draft.blocked_mode == BlockedMode::Hide {
            "Filtrés masqués"
        } else {
            "Masquer les filtrés"
        })
        .on_press(Message::Settings(SettingsMessage::BlockedMode(
            BlockedMode::Hide
        )))
        .style(colors::button_secondary),
        checkbox(draft.show_hidden)
            .label("Afficher aussi les posts masqués")
            .on_toggle(|value| Message::Settings(SettingsMessage::ShowHidden(value)))
            .size(16),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let notifications = column![
        row![
            checkbox(draft.notifications.enabled)
                .label("Notifications de bureau")
                .on_toggle(|value| Message::Settings(SettingsMessage::NotifyEnabled(value)))
                .size(16),
            checkbox(draft.notifications.only_unfocused)
                .label("Seulement quand roinroin n'est pas au premier plan (sauf bigornos)")
                .on_toggle(|value| Message::Settings(SettingsMessage::NotifyOnlyUnfocused(value)))
                .size(16),
        ]
        .spacing(14),
        row![
            checkbox(draft.notifications.bigorno)
                .label("Bigornos")
                .on_toggle(|value| Message::Settings(SettingsMessage::NotifyBigorno(value)))
                .size(16),
            checkbox(draft.notifications.replies)
                .label("Réponses à mes posts")
                .on_toggle(|value| Message::Settings(SettingsMessage::NotifyReplies(value)))
                .size(16),
            checkbox(draft.notifications.all_posts)
                .label("Tous les nouveaux posts")
                .on_toggle(|value| Message::Settings(SettingsMessage::NotifyAll(value)))
                .size(16),
        ]
        .spacing(14),
    ]
    .spacing(6);

    let linuxfr = column![
        row![
            text_input("jeton ou URL de retour LinuxFR", &draft.linuxfr_paste)
                .on_input(|value| Message::Settings(SettingsMessage::LinuxfrPaste(value)))
                .width(Fill)
                .style(colors::text_input_style),
            button("Ouvrir l'autorisation")
                .on_press(Message::OpenUrl(
                    app.config
                        .servers
                        .iter()
                        .find(|server| server.enabled)
                        .map(|server| {
                            format!(
                                "{}/gb2c/linuxfr/authorize",
                                server.url.trim_end_matches('/')
                            )
                        })
                        .unwrap_or_default(),
                ))
                .style(colors::button_secondary),
            button("Rafraîchir")
                .on_press(Message::RefreshLinuxfr)
                .style(colors::button_ghost),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        text(
            "gb2c ne peut pas renvoyer le jeton au client : ouvre la page d'autorisation, \
             puis colle ici l'URL de retour (ou le jeton) pour que roinroin puisse poster sur LinuxFR."
        )
        .size(10)
        .style(colors::muted_text),
        text(match app.config.linuxfr.as_ref() {
            Some(session) if !session.access_token.is_empty() => {
                format!(
                    "Connecté{}",
                    if session.login.is_empty() {
                        String::new()
                    } else {
                        format!(" comme {}", session.login)
                    }
                )
            }
            _ => "Non connecté".to_owned(),
        })
        .size(11)
        .style(colors::muted_text),
    ]
    .spacing(6);

    let mut tribune_list = column![].spacing(4);
    let mut tribune_count = 0usize;
    for server in app.config.servers.iter().filter(|server| server.enabled) {
        let url = crate::config::normalize_url(&server.url).unwrap_or_else(|| server.url.clone());
        if let Some(state) = app.servers.get(&url) {
            for name in &state.known {
                let key = TribuneKey::new(SourceId::Gb2c(url.clone()), name.clone());
                let key_string = key.key();
                let visible = !draft.hidden_tribunes.contains(&key_string);
                let label = format!("{} / {}", server.name, name);
                tribune_count += 1;
                tribune_list = tribune_list.push(
                    checkbox(visible)
                        .label(label)
                        .on_toggle(move |_| {
                            Message::Settings(SettingsMessage::ToggleTribune(key_string.clone()))
                        })
                        .size(16),
                );
            }
        }
    }
    if tribune_count == 0 {
        tribune_list = tribune_list.push(
            text("Aucune tribune gb2c connue pour le moment.")
                .size(11)
                .style(colors::muted_text),
        );
    }

    let mut page = column![row![
        text("Réglages").size(28).font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::DEFAULT
        }),
        Space::new().width(Fill),
        button("Annuler")
            .on_press(Message::Settings(SettingsMessage::Cancel))
            .style(colors::button_ghost),
        button("Appliquer")
            .on_press(Message::Settings(SettingsMessage::Apply))
            .style(colors::button_primary),
    ]
    .align_y(Alignment::Center)
    .width(Fill),]
    .spacing(14)
    .padding(18)
    .max_width(980);

    if let Some(error) = &draft.error {
        page = page.push(text(error.clone()).style(colors::danger_text));
    }
    if let Some(info) = &draft.info {
        page = page.push(text(info.clone()).style(colors::success_text));
    }

    page = page
        .push(section(
            "Identité",
            column![
                row![
                    text("Pseudo").width(Length::Fixed(130.0)),
                    text_input("pseudo envoyé avec les posts", &draft.nickname)
                        .on_input(|value| Message::Settings(SettingsMessage::Nickname(value)))
                        .width(Fill)
                        .style(colors::text_input_style),
                ]
                .align_y(Alignment::Center),
                row![
                    text("ToToz").width(Length::Fixed(130.0)),
                    text_input("serveur totoz", &draft.totoz_server)
                        .on_input(|value| Message::Settings(SettingsMessage::TotozServer(value)))
                        .width(Fill)
                        .style(colors::text_input_style),
                ]
                .align_y(Alignment::Center),
                row![
                    text("Thème").width(Length::Fixed(130.0)),
                    button("Sombre")
                        .on_press(Message::Settings(SettingsMessage::Theme(ThemeMode::Dark)))
                        .style(colors::button_secondary),
                    button("Clair")
                        .on_press(Message::Settings(SettingsMessage::Theme(ThemeMode::Light)))
                        .style(colors::button_secondary),
                    button("Système")
                        .on_press(Message::Settings(SettingsMessage::Theme(ThemeMode::System)))
                        .style(colors::button_secondary),
                    text("Langue").size(12),
                    text("Français (English à venir)")
                        .size(12)
                        .style(colors::muted_text),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        ))
        .push(section("Serveurs gb2c", servers))
        .push(section("Tribunes gb2c visibles", tribune_list))
        .push(section(
            "Boards directs",
            column![board_list, add_board].spacing(10),
        ))
        .push(section(
            "Filtrage",
            column![
                blocked_mode,
                row![
                    text("Identités bloquées").width(Length::Fixed(160.0)),
                    text_input("pseudo1, pseudo2, …", &draft.blocked)
                        .on_input(|value| Message::Settings(SettingsMessage::Blocked(value)))
                        .width(Fill)
                        .style(colors::text_input_style),
                ]
                .align_y(Alignment::Center),
                row![
                    text("Liste plop").width(Length::Fixed(160.0)),
                    text_input("auteurs remplacés par plop/pika/prout", &draft.plop)
                        .on_input(|value| Message::Settings(SettingsMessage::Plop(value)))
                        .width(Fill)
                        .style(colors::text_input_style),
                ]
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        ))
        .push(section("Notifications", notifications))
        .push(section("LinuxFR", linuxfr))
        .push(Space::new().height(20));

    scrollable(container(page).width(Fill).padding(18))
        .height(Fill)
        .into()
}

fn draft_boards(draft: &crate::app::SettingsDraft) -> Vec<crate::config::Board> {
    let mut boards = crate::config::predefined_boards();
    let names: BTreeMap<String, ()> = boards
        .iter()
        .map(|board| (board.name.clone(), ()))
        .collect();
    for board in &draft.custom_boards {
        if board.validate().is_ok() {
            let board = board.normalized();
            if !names.contains_key(&board.name) {
                boards.push(board);
            }
        }
    }
    boards.sort_by(|a, b| a.name.cmp(&b.name));
    boards
}

fn section<'a>(title: &'a str, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(
        column![
            text(title.to_uppercase())
                .size(11)
                .style(colors::muted_text),
            content.into(),
        ]
        .spacing(10),
    )
    .style(colors::panel)
    .padding(14)
    .width(Fill)
    .into()
}

// ---------------------------------------------------------------------------
// ToToz

fn totoz_view(app: &App) -> Element<'_, Message> {
    let mut results = column![].spacing(8).width(Fill);
    if app.totoz.loading {
        results = results.push(text("Recherche…").style(colors::muted_text));
    }
    if let Some(error) = &app.totoz.error {
        results = results.push(text(error.clone()).style(colors::danger_text));
    }
    for chunk in app.totoz.results.chunks(5) {
        let mut line = row![].spacing(8).width(Fill);
        for totoz in chunk {
            let preview: Element<'_, Message> =
                if let Some(preview) = app.totoz.images.get(&totoz.name) {
                    image(preview.first_frame().clone())
                        .width(96)
                        .height(96)
                        .content_fit(iced::ContentFit::Contain)
                        .into()
                } else {
                    container(
                        container(text("…").size(24).style(colors::muted_text))
                            .center_x(Fill)
                            .center_y(Fill),
                    )
                    .width(96)
                    .height(96)
                    .style(colors::card)
                    .into()
                };
            line = line.push(
                button(
                    column![
                        preview,
                        text(totoz.name.clone())
                            .size(11)
                            .width(Length::Fixed(96.0))
                            .wrapping(iced::widget::text::Wrapping::Word),
                    ]
                    .spacing(4)
                    .width(Length::Fixed(104.0))
                    .align_x(Alignment::Center),
                )
                .on_press(Message::TotozPicked(totoz.name.clone()))
                .style(colors::button_ghost)
                .padding(4),
            );
        }
        results = results.push(line);
    }
    if app.totoz.results.is_empty() && !app.totoz.loading {
        results = results.push(
            text("Cherche un totoz puis clique dessus pour l'insérer.")
                .size(13)
                .style(colors::muted_text),
        );
    }

    let header = row![
        button("← Retour")
            .on_press(Message::CloseScreen)
            .style(colors::button_ghost),
        text("ToToz").size(26).font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::DEFAULT
        }),
        text(format!("{} résultats", app.totoz.results.len()))
            .size(12)
            .style(colors::muted_text),
        Space::new().width(Fill),
        text(format!("serveur : {}", app.config.totoz_server))
            .size(11)
            .style(colors::muted_text),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .width(Fill);

    let search = row![
        text_input("chercher des totoz", &app.totoz.query)
            .id(app.totoz_search_id.clone())
            .on_input(Message::TotozQuery)
            .on_submit(Message::TotozSearch)
            .width(Fill)
            .style(colors::text_input_style),
        button("Chercher")
            .on_press(Message::TotozSearch)
            .style(colors::button_primary),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Fill);

    container(
        column![
            header,
            search,
            scrollable(container(results).width(Fill).padding(4)).height(Fill)
        ]
        .spacing(12)
        .padding(18)
        .height(Fill)
        .width(Fill),
    )
    .width(Fill)
    .height(Fill)
    .into()
}

// ---------------------------------------------------------------------------
// Emoji

fn emoji_view(app: &App) -> Element<'_, Message> {
    let mut grid_items: Vec<Element<'_, Message>> = Vec::new();
    for emoji in &app.emoji.results {
        grid_items.push(
            button(
                column![
                    text(emoji.characters.clone()).size(28),
                    text(emoji.name.clone())
                        .size(9)
                        .width(Length::Fixed(84.0))
                        .wrapping(iced::widget::text::Wrapping::Word)
                        .style(colors::muted_text),
                ]
                .spacing(2)
                .align_x(Alignment::Center),
            )
            .on_press(Message::Insert(emoji.characters.clone()))
            .style(colors::button_ghost)
            .width(Length::Fixed(92.0))
            .padding(6)
            .into(),
        );
    }
    let mut columns = column![].spacing(6).width(Fill);
    let mut items = grid_items.into_iter();
    loop {
        let mut line = row![].spacing(4).width(Fill);
        let mut count = 0usize;
        for item in items.by_ref().take(8) {
            line = line.push(item);
            count += 1;
        }
        if count == 0 {
            break;
        }
        columns = columns.push(line);
    }

    let header = row![
        button("← Retour")
            .on_press(Message::CloseScreen)
            .style(colors::button_ghost),
        text("Emoji").size(26).font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::DEFAULT
        }),
        Space::new().width(Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .width(Fill);

    let search = row![
        text_input("chercher un emoji", &app.emoji.query)
            .id(app.emoji_search_id.clone())
            .on_input(Message::EmojiQuery)
            .on_submit(Message::EmojiSearch)
            .width(Fill)
            .style(colors::text_input_style),
        button("Chercher")
            .on_press(Message::EmojiSearch)
            .style(colors::button_primary),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Fill);

    container(
        column![
            header,
            search,
            scrollable(container(columns).width(Fill).padding(4)).height(Fill)
        ]
        .spacing(12)
        .padding(18)
        .height(Fill)
        .width(Fill),
    )
    .width(Fill)
    .height(Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::widget::Tree;
    use iced::widget::{column, text_input};

    type Paragraph = <iced::Renderer as iced::advanced::text::Renderer>::Paragraph;

    /// Mirror the composer structure: toolbar, suggestion slot, entry field.
    fn composer_with_suggestions(
        suggestions: Option<Element<'static, ()>>,
    ) -> Element<'static, ()> {
        let slot: Element<'static, ()> =
            suggestions.unwrap_or_else(|| Space::new().height(Length::Shrink).into());
        column![
            text("toolbar"),
            slot,
            row![text_input("hint", "hello").id(iced::widget::Id::new("composer"))],
        ]
        .into()
    }

    #[test]
    fn entry_field_keeps_focus_when_the_suggestion_slot_changes() {
        let mut tree = Tree::new(composer_with_suggestions(None).as_widget());

        // The placeholders are kept in the tree, so the entry field is always
        // the third column child and can be focused directly.
        let entry = &mut tree.children[2].children[0];
        entry
            .state
            .downcast_mut::<text_input::State<Paragraph>>()
            .focus();

        let slot: Element<'static, ()> = container(column![text("[:duck]")]).height(20).into();
        tree.diff(composer_with_suggestions(Some(slot)).as_widget());

        let entry = &tree.children[2].children[0];
        let state = entry.state.downcast_ref::<text_input::State<Paragraph>>();
        assert!(state.is_focused(), "the entry field lost its focus state");
    }
}
