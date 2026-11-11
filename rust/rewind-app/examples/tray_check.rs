//! Create the real tray, render every state, and check menu clicks map to actions.
//!
//! This runs on the process main thread, which `muda` requires for menus — the reason
//! it is an example rather than a `cargo test` case (the test harness runs each test
//! on a worker thread).
//!
//! `cargo run -p rewind-app --features tray --example tray_check`

#[cfg(feature = "tray")]
fn main() {
    use rewind_app::tray::{TrayAction, TrayState};
    use rewind_app::tray_ui::{action_for_menu_id, poll_menu_events, Tray};

    let logo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/logo.png");
    let bytes = std::fs::read(&logo).expect("logo.png");
    let (rgba, w, h) = rewind_app::tray_ui::decode_png(&bytes).expect("logo decodes");
    let opaque = rgba.chunks_exact(4).filter(|p| p[3] > 0).count();
    println!(
        "logo: {w}x{h}, {} rgba bytes, {opaque} opaque pixels",
        rgba.len()
    );
    assert_eq!(rgba.len(), (w * h * 4) as usize);
    assert!(opaque > 1000, "logo looks blank");

    let tray = Tray::new(TrayState::Off, Some(&logo)).expect("tray created");
    println!("tray created with {} menu rows", tray.menu().items().len());
    assert_eq!(tray.menu().items().len(), 9, "7 actions plus 2 separators");

    for state in [
        TrayState::On,
        TrayState::Recording,
        TrayState::Saving,
        TrayState::Starting,
        TrayState::Off,
    ] {
        tray.render(state);
        println!("  rendered: {}", rewind_app::tray::tooltip(state));
        std::thread::sleep(std::time::Duration::from_millis(150));
    }

    for (id, expected) in [
        ("save_clip", Some(TrayAction::SaveClip)),
        ("toggle_replay", Some(TrayAction::ToggleReplay)),
        ("quit", Some(TrayAction::Quit)),
        ("nonsense", None),
    ] {
        let mapped = action_for_menu_id(&muda::MenuId(id.to_string()));
        println!("  click {id:?} -> {mapped:?}");
        assert_eq!(mapped, expected, "menu id {id} mapped wrong");
    }
    let _ = poll_menu_events();

    // A missing icon must not stop the tray from coming up.
    let fallback = Tray::new(TrayState::Off, Some(std::path::Path::new("/nope.png")));
    println!(
        "tray without icon: {}",
        if fallback.is_ok() { "ok" } else { "failed" }
    );
    assert!(fallback.is_ok(), "a missing icon should not stop the tray");

    println!("tray ok");
}

#[cfg(not(feature = "tray"))]
fn main() {
    println!("needs --features tray");
}
