//! roinroin — native Linux tribune client.
//!
//! The UI is built with iced and the networking layer talks both to a gb2c
//! server (HTTP/SSE) and directly to bouchot boards (TSV/XML + form POST).

mod app;
mod composer_input;
mod config;
mod conversation;
mod hover_text;
mod message;
mod model;
mod net;
mod notify;
mod theme;
mod tsv;
mod ui;
mod visible;

use iced::window;
use iced::Size;

fn main() -> iced::Result {
    if !parse_args() {
        return Ok(());
    }
    let icon = window_icon();
    iced::application(app::App::new, app::App::update, ui::view)
        .title(app::App::title)
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .window_size(Size::new(1180.0, 820.0))
        .window(window::Settings {
            size: Size::new(1180.0, 820.0),
            min_size: Some(Size::new(820.0, 540.0)),
            icon,
            platform_specific: window::settings::PlatformSpecific {
                application_id: "roinroin".to_owned(),
                ..window::settings::PlatformSpecific::default()
            },
            ..window::Settings::default()
        })
        .antialiasing(true)
        .run()
}

/// Minimal command line, kept dependency-free. Values are passed through
/// environment variables to the configuration loader.
fn parse_args() -> bool {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "roinroin {}\n\nUsage: roinroin [options]\n\nOptions:\n  --gb2c URL, --url URL   use this gb2c server\n  --nickname NAME         nickname sent with posts\n  --no-notifications      disable desktop notifications\n  --version, -V           print version\n  --help, -h              this help\n\nThe persistent configuration lives in\n$XDG_CONFIG_HOME/roinroin/config.toml.",
                    env!("CARGO_PKG_VERSION")
                );
                return false;
            }
            "--version" | "-V" => {
                println!("roinroin {}", env!("CARGO_PKG_VERSION"));
                return false;
            }
            "--gb2c" | "--url" => {
                if let Some(value) = args.next() {
                    std::env::set_var("ROINROIN_GB2C", value);
                }
            }
            "--nickname" | "--info" => {
                if let Some(value) = args.next() {
                    std::env::set_var("ROINROIN_NICKNAME", value);
                }
            }
            "--no-notifications" => {
                std::env::set_var("ROINROIN_NO_NOTIFICATIONS", "1");
            }
            other => {
                eprintln!("roinroin: option inconnue : {other}");
                eprintln!("Essaie `roinroin --help`.");
                std::process::exit(2);
            }
        }
    }
    true
}

/// Generate a small RGBA icon at runtime; this avoids shipping a binary asset
/// just for the window manager while still giving the app a distinctive mark.
fn window_icon() -> Option<window::Icon> {
    const SIZE: u32 = 64;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - 32.0;
            let dy = y as f32 - 32.0;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance > 30.0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // A small duck-ish gradient: violet plate, amber beak.
            let beak = (14..=34).contains(&x) && (30..=36).contains(&y);
            let (r, g, b) = if beak {
                (255, 196, 64)
            } else if distance < 22.0 {
                (150, 112, 255)
            } else {
                (90, 64, 200)
            };
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    window::icon::from_rgba(rgba, SIZE, SIZE).ok()
}
