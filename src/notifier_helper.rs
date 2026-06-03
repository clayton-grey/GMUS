use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{bail, Context, Result};
use cocoa::appkit::{
    NSApp, NSApplication, NSApplicationActivationPolicyAccessory, NSBackingStoreBuffered, NSColor,
    NSEventMask, NSImage, NSImageView, NSMainMenuWindowLevel, NSScreen, NSTextField, NSView,
    NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::{
    NSAutoreleasePool, NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize, NSString,
};
use objc::{class, msg_send, sel, sel_impl};

pub(crate) const HELPER_MODE_ARG: &str = "--gmus-notifier";

const TITLE_ARG: &str = "--title";
const SUBTITLE_ARG: &str = "--subtitle";
const BODY_ARG: &str = "--body";
const ARTWORK_ARG: &str = "--artwork";

const OVERLAY_VISIBLE_FOR: Duration = Duration::from_millis(2_800);
const OVERLAY_FADE_IN: Duration = Duration::from_millis(120);
const OVERLAY_FADE_OUT: Duration = Duration::from_millis(180);
const OVERLAY_FRAME_INTERVAL: Duration = Duration::from_millis(16);

pub(crate) fn run_if_requested() -> Result<bool> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let Some(helper_index) = args
        .iter()
        .position(|arg| arg.as_os_str() == OsStr::new(HELPER_MODE_ARG))
    else {
        return Ok(false);
    };

    if let Err(error) = run(args.into_iter().skip(helper_index + 1)) {
        log_helper_error(&error);
        return Err(error);
    }

    Ok(true)
}

pub(crate) fn notification_args(
    title: &str,
    subtitle: Option<&str>,
    body: &str,
    artwork_path: Option<&Path>,
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from(HELPER_MODE_ARG),
        OsString::from(TITLE_ARG),
        OsString::from(title),
        OsString::from(BODY_ARG),
        OsString::from(body),
    ];

    if let Some(subtitle) = subtitle {
        args.push(OsString::from(SUBTITLE_ARG));
        args.push(OsString::from(subtitle));
    }

    if let Some(artwork_path) = artwork_path {
        args.push(OsString::from(ARTWORK_ARG));
        args.push(artwork_path.as_os_str().to_os_string());
    }

    args
}

pub(crate) fn gmus_support_dir() -> Result<PathBuf> {
    let home = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .context("HOME is not set")?;
    Ok(home
        .join("Library")
        .join("Application Support")
        .join("GMUS"))
}

fn run(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let payload = HelperPayload::parse(args)?;
    show_overlay(&payload)
}

fn show_overlay(payload: &HelperPayload) -> Result<()> {
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        let result = show_overlay_with_pool(payload);
        pool.drain();
        result
    }
}

unsafe fn show_overlay_with_pool(payload: &HelperPayload) -> Result<()> {
    let app = prepare_accessory_app();
    let overlay = OverlayWindow::new(payload)?;
    overlay.show();
    fade_window(overlay.window, 0.0, 1.0, OVERLAY_FADE_IN, app);
    pump_app_for(OVERLAY_VISIBLE_FOR, app);
    fade_window(overlay.window, 1.0, 0.0, OVERLAY_FADE_OUT, app);
    overlay.close();
    Ok(())
}

struct OverlayWindow {
    window: id,
}

impl OverlayWindow {
    unsafe fn new(payload: &HelperPayload) -> Result<Self> {
        let screen = NSScreen::mainScreen(nil);
        if screen == nil {
            bail!("no main screen available for GMUS overlay");
        }

        let has_artwork = overlay_artwork_path(payload.artwork_path.as_deref()).is_some();
        let layout = OverlayLayout::new(has_artwork);
        let frame = layout.window_frame(screen.visibleFrame());
        let window = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
            frame,
            NSWindowStyleMask::NSBorderlessWindowMask,
            NSBackingStoreBuffered,
            NO,
        );
        if window == nil {
            bail!("failed to create GMUS overlay window");
        }

        window.setOpaque_(NO);
        window.setBackgroundColor_(NSColor::clearColor(nil));
        window.setHasShadow_(YES);
        window.setCanHide_(NO);
        window.setHidesOnDeactivate_(NO);
        window.setLevel_(NSMainMenuWindowLevel as i64 + 1);
        window.setCollectionBehavior_(
            NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorTransient
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary,
        );
        let _: () = msg_send![window, setIgnoresMouseEvents: YES];

        let content = NSView::initWithFrame_(NSView::alloc(nil), layout.content_frame());
        configure_rounded_layer(
            content,
            NSColor::colorWithSRGBRed_green_blue_alpha_(nil, 0.05, 0.055, 0.06, 0.92),
            18.0,
        );
        window.setContentView_(content);

        if let Some(artwork_path) = overlay_artwork_path(payload.artwork_path.as_deref()) {
            let image = load_image(artwork_path);
            if image != nil {
                let image_view =
                    NSImageView::initWithFrame_(NSImageView::alloc(nil), layout.art_frame);
                image_view.setImage_(image);
                let _: () = msg_send![image_view, setImageScaling: 3_u64];
                configure_rounded_layer(image_view, NSColor::clearColor(nil), 12.0);
                content.addSubview_(image_view);
            }
        }

        add_label(content, layout.title_frame, &payload.title, 17.0, true, 1.0);
        if let Some(subtitle) = &payload.subtitle {
            add_label(content, layout.subtitle_frame, subtitle, 13.0, false, 0.78);
        }
        if !payload.body.is_empty() {
            add_label(content, layout.body_frame, &payload.body, 12.0, false, 0.58);
        }

        window.setAlphaValue_(0.0);
        Ok(Self { window })
    }

    unsafe fn show(&self) {
        self.window.orderFrontRegardless();
    }

    unsafe fn close(&self) {
        self.window.orderOut_(nil);
    }
}

#[derive(Clone, Copy)]
struct OverlayLayout {
    size: NSSize,
    art_frame: NSRect,
    title_frame: NSRect,
    subtitle_frame: NSRect,
    body_frame: NSRect,
}

impl OverlayLayout {
    fn new(has_artwork: bool) -> Self {
        let size = NSSize::new(440.0, 136.0);
        let padding = 14.0;
        let art_size = if has_artwork { 108.0 } else { 0.0 };
        let text_x = if has_artwork {
            padding + art_size + 14.0
        } else {
            padding
        };
        let text_width = size.width - text_x - padding;

        Self {
            size,
            art_frame: NSRect::new(
                NSPoint::new(padding, padding),
                NSSize::new(art_size, art_size),
            ),
            title_frame: NSRect::new(NSPoint::new(text_x, 84.0), NSSize::new(text_width, 28.0)),
            subtitle_frame: NSRect::new(NSPoint::new(text_x, 59.0), NSSize::new(text_width, 21.0)),
            body_frame: NSRect::new(NSPoint::new(text_x, 36.0), NSSize::new(text_width, 19.0)),
        }
    }

    fn content_frame(&self) -> NSRect {
        NSRect::new(NSPoint::new(0.0, 0.0), self.size)
    }

    fn window_frame(&self, visible_frame: NSRect) -> NSRect {
        let margin = 20.0;
        let x = visible_frame.origin.x + visible_frame.size.width - self.size.width - margin;
        let y = visible_frame.origin.y + visible_frame.size.height - self.size.height - margin;
        NSRect::new(NSPoint::new(x, y), self.size)
    }
}

unsafe fn prepare_accessory_app() -> id {
    let app = NSApp();
    let _ = app.setActivationPolicy_(NSApplicationActivationPolicyAccessory);
    app.finishLaunching();
    app
}

unsafe fn add_label(parent: id, frame: NSRect, text: &str, font_size: f64, bold: bool, alpha: f64) {
    let field = NSTextField::initWithFrame_(NSTextField::alloc(nil), frame);
    field.setEditable_(NO);
    field.setStringValue_(NSString::alloc(nil).init_str(text));

    let _: () = msg_send![field, setBezeled: NO];
    let _: () = msg_send![field, setBordered: NO];
    let _: () = msg_send![field, setDrawsBackground: NO];
    let _: () = msg_send![field, setSelectable: NO];
    let _: () = msg_send![field, setTextColor: NSColor::colorWithSRGBRed_green_blue_alpha_(nil, 1.0, 1.0, 1.0, alpha)];
    let font: id = if bold {
        msg_send![class!(NSFont), boldSystemFontOfSize: font_size]
    } else {
        msg_send![class!(NSFont), systemFontOfSize: font_size]
    };
    let _: () = msg_send![field, setFont: font];
    let cell: id = msg_send![field, cell];
    let _: () = msg_send![cell, setLineBreakMode: 4_u64];
    let _: () = msg_send![cell, setTruncatesLastVisibleLine: YES];

    parent.addSubview_(field);
}

unsafe fn configure_rounded_layer(view: id, color: id, radius: f64) {
    view.setWantsLayer(YES);
    let layer = view.layer();
    if layer == nil {
        return;
    }

    let background: id = msg_send![color, CGColor];
    let _: () = msg_send![layer, setBackgroundColor: background];
    let _: () = msg_send![layer, setCornerRadius: radius];
    let _: () = msg_send![layer, setMasksToBounds: YES];
}

unsafe fn load_image(path: &Path) -> id {
    let image_path = NSString::alloc(nil).init_str(&path.to_string_lossy());
    NSImage::alloc(nil).initWithContentsOfFile_(image_path)
}

fn overlay_artwork_path(artwork_path: Option<&Path>) -> Option<&Path> {
    artwork_path.filter(|path| path.is_file())
}

unsafe fn fade_window(window: id, from: f64, to: f64, duration: Duration, app: id) {
    if duration.is_zero() {
        window.setAlphaValue_(to);
        return;
    }

    let started = Instant::now();
    while started.elapsed() < duration {
        let progress = (started.elapsed().as_secs_f64() / duration.as_secs_f64()).min(1.0);
        window.setAlphaValue_(from + (to - from) * progress);
        pump_app_for(OVERLAY_FRAME_INTERVAL, app);
    }
    window.setAlphaValue_(to);
}

unsafe fn pump_app_for(duration: Duration, app: id) {
    let started = Instant::now();
    while started.elapsed() < duration {
        pump_pending_app_events(app);
        std::thread::sleep(OVERLAY_FRAME_INTERVAL.min(duration.saturating_sub(started.elapsed())));
    }
    pump_pending_app_events(app);
}

unsafe fn pump_pending_app_events(app: id) {
    let until = NSDate::distantPast(nil);
    loop {
        let event = app.nextEventMatchingMask_untilDate_inMode_dequeue_(
            NSEventMask::NSAnyEventMask.bits(),
            until,
            NSDefaultRunLoopMode,
            YES,
        );
        if event == nil {
            break;
        }
        app.sendEvent_(event);
    }
}

fn log_helper_error(error: &anyhow::Error) {
    let Ok(dir) = gmus_support_dir() else {
        return;
    };
    if fs::create_dir_all(&dir).is_err() {
        return;
    }

    let path = dir.join("notifier.log");
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };

    let _ = writeln!(
        file,
        "{:?} notifier helper failed: {error:#}",
        SystemTime::now()
    );
}

#[derive(Debug, PartialEq, Eq)]
struct HelperPayload {
    title: String,
    subtitle: Option<String>,
    body: String,
    artwork_path: Option<PathBuf>,
}

impl HelperPayload {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self> {
        let mut title = None;
        let mut subtitle = None;
        let mut body = None;
        let mut artwork_path = None;

        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let flag = flag
                .into_string()
                .map_err(|_| anyhow::anyhow!("notifier argument names must be UTF-8"))?;
            if flag.starts_with("-psn_") {
                continue;
            }

            let value = args
                .next()
                .with_context(|| format!("missing value for notifier argument {flag}"))?;

            match flag.as_str() {
                TITLE_ARG => title = Some(utf8_value(TITLE_ARG, value)?),
                SUBTITLE_ARG => subtitle = Some(utf8_value(SUBTITLE_ARG, value)?),
                BODY_ARG => body = Some(utf8_value(BODY_ARG, value)?),
                ARTWORK_ARG => artwork_path = Some(PathBuf::from(value)),
                _ => bail!("unknown notifier argument {flag}"),
            }
        }

        Ok(Self {
            title: title.context("missing notifier title")?,
            subtitle,
            body: body.context("missing notifier body")?,
            artwork_path,
        })
    }
}

fn utf8_value(name: &str, value: OsString) -> Result<String> {
    value
        .into_string()
        .map_err(|_| anyhow::anyhow!("{name} must be valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use std::fs;

    use super::{notification_args, overlay_artwork_path, HelperPayload};

    #[test]
    fn notification_args_round_trip_payload() {
        let args = notification_args(
            "Song",
            Some("Artist"),
            "Album",
            Some(Path::new("/tmp/cover art.jpg")),
        );

        let payload = HelperPayload::parse(args.into_iter().skip(1)).unwrap();

        assert_eq!(
            payload,
            HelperPayload {
                title: String::from("Song"),
                subtitle: Some(String::from("Artist")),
                body: String::from("Album"),
                artwork_path: Some(Path::new("/tmp/cover art.jpg").to_path_buf()),
            }
        );
    }

    #[test]
    fn parse_rejects_missing_title() {
        let error =
            HelperPayload::parse([OsString::from("--body"), OsString::from("Album")]).unwrap_err();

        assert!(format!("{error:#}").contains("missing notifier title"));
    }

    #[test]
    fn parse_ignores_launch_services_process_serial_number() {
        let payload = HelperPayload::parse([
            OsString::from("-psn_0_12345"),
            OsString::from("--title"),
            OsString::from("Song"),
            OsString::from("--body"),
            OsString::from("Album"),
        ])
        .unwrap();

        assert_eq!(payload.title, "Song");
        assert_eq!(payload.body, "Album");
    }

    #[test]
    fn overlay_artwork_path_requires_file() {
        let temp = tempfile::tempdir().unwrap();
        let image_path = temp.path().join("cover.jpg");
        fs::write(&image_path, "not a real jpeg but enough for the path gate").unwrap();

        assert_eq!(
            overlay_artwork_path(Some(&image_path)),
            Some(image_path.as_path())
        );
        assert_eq!(overlay_artwork_path(Some(temp.path())), None);
        assert_eq!(overlay_artwork_path(None), None);
    }
}
