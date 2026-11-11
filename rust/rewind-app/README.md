# Rewind shell (Rust)

The desktop half of Rewind: one window showing the Clips page, a tray icon, and global
hotkeys. The capture engine lives in `rewind-engine`; nothing here touches FFmpeg
directly.

```
rewind-app/
├── src/
│   ├── controller.rs   engine ↔ UI glue, clip listing, settings, notices
│   ├── hotkeys.rs      RegisterHotKey (Windows) / RegisterEventHotKey (macOS)
│   ├── tray.rs         pure menu + tooltip model
│   ├── tray_ui.rs      renders that model with tray-icon/muda
│   ├── event_loop.rs   the main thread's Windows message pump
│   ├── overlay*.rs     status-pill content model and capture exclusion
│   ├── clipboard.rs    file lists on the clipboard, for pasting into chat apps
│   └── ui/             the window: Clips page, modals, theme
└── src/bin/rewind.rs   wiring: tray, hotkeys, then the window
```

## The app loop

`eframe` owns the loop. `App::logic` does the shell work — hotkey presses, tray clicks,
engine events, tray re-rendering — and runs *while the window is hidden*, which is what
keeps a closed window capturing. `App::ui` draws.

Closing the window hides it rather than quitting, because Rewind lives in the tray. With
the `tray` feature off there is no way back in, so closing quits instead.

## Platform rules that bite

* **The tray needs a message pump on Windows.** A tray icon is a window; without a pump
  the icon appears and is deaf — clicks, menu selections and the `TaskbarCreated`
  re-add after an Explorer restart all sit unread. `event_loop::pump` drains the queue
  once per tick.
* **Hotkeys own their queue.** On Windows `RegisterHotKey` posts `WM_HOTKEY` to the
  sticky thread that registered it, so that thread pumps its own queue; on macOS Carbon
  dispatches through the main run loop. Presses are forwarded to the UI thread, which
  requests a repaint so a press is handled immediately, not on the next poll.
* **Menus are built on the main thread** on both platforms, before the window opens.

## Clipboard

Copying a clip means putting a *file list* on the clipboard, not a path string: chat
apps ask for `CF_HDROP` on Windows and `public.file-url` on macOS.

Two macOS traps, both found by running it:

1. Linking `libobjc` is not enough. `objc_getClass("NSString")` answers nil until
   Foundation and AppKit are loaded, so both frameworks are linked explicitly.
2. Passing live `NSURL` objects to `writeObjects:` only *promises* them. Draining the
   autorelease pool — or exiting right afterwards — leaves the clipboard empty. The
   items here are `NSPasteboardItem`s holding copied strings instead.

## Run and verify

```
cargo run -p rewind-app --features tray
cargo test -p rewind-app
cargo clippy --target x86_64-pc-windows-msvc --workspace --all-targets \
  --features rewind-app/tray -- -D warnings
```

The last one type-checks every `cfg(windows)` path — tray, hotkeys, clipboard — from a
machine that is not Windows. It does not replace running the app there.
