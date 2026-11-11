//! Global hotkeys without keyboard hooks or extra permissions.
//!
//! Windows uses `RegisterHotKey`; macOS uses Carbon `RegisterEventHotKey`. Both are
//! the same APIs the Python app used, so behaviour — including which combos macOS
//! refuses — carries over exactly.
//!
//! The two platforms deliver presses very differently, and that shapes this module:
//!
//! * **Windows** needs a thread with its own message queue. `RegisterHotKey` posts
//!   `WM_HOTKEY` to it, so registration and the message pump must share a thread.
//!   Registering from one thread and pumping on another silently never fires.
//! * **macOS** dispatches hotkey events through the process's Carbon event target,
//!   which is served by the main run loop. There is no message queue to own; the
//!   handlers fire while the app runs its loop.
//!
//! Because a hotkey is useless if the event loop stops, [`HotkeyManager::status`]
//! reports per-name registration success, so a combo another app already owns can be
//! surfaced in settings instead of failing silently.

#[cfg(target_os = "windows")]
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
#[cfg(target_os = "windows")]
use std::time::Duration;

use rewind_engine::settings::{Hotkey, Settings};

use backend::Backend;

/// Names the app reacts to, in the order the tray and settings page list them.
pub const HOTKEY_ACTIONS: [&str; 5] = ["toggle", "clip", "record", "screenshot", "open"];

/// Per-action registration outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registration {
    /// Bound and ready.
    Ok,
    /// Another application already owns the combo, or the OS refused it.
    Taken,
    /// No combo configured for this action.
    Unset,
}

impl Registration {
    pub fn is_ok(self) -> bool {
        matches!(self, Registration::Ok)
    }
}

/// Why the manager could not start at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    /// The platform backend is unavailable (e.g. Carbon could not be loaded).
    Unsupported(String),
    /// The OS refused to create the message queue or worker thread.
    Backend(String),
}

impl std::fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HotkeyError::Unsupported(m) | HotkeyError::Backend(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for HotkeyError {}

/// A hotkey press, delivered to the app's handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Triggered {
    pub action: String,
}

/// Collects the actions configured in `Settings`, dropping duplicates.
///
/// Two actions bound to the same combo would fight over one registration, and the
/// second would silently never fire, so the later one is reported as [`Registration::Taken`]
/// rather than registered.
pub fn configured_hotkeys(settings: &Settings) -> (Vec<(String, Hotkey)>, Vec<String>) {
    let pairs: Vec<(String, Hotkey)> = HOTKEY_ACTIONS
        .iter()
        .filter_map(|name| action_hotkey(settings, name).map(|hk| ((*name).to_string(), hk)))
        .collect();

    let mut kept: Vec<(String, Hotkey)> = Vec::new();
    let mut duplicates: Vec<String> = Vec::new();
    for (name, hk) in pairs {
        // An unset hotkey collides with nothing: several actions may be blank.
        if hk.vk != 0
            && kept
                .iter()
                .any(|(_, other)| other.vk != 0 && other.combo() == hk.combo())
        {
            duplicates.push(name);
        } else {
            kept.push((name, hk));
        }
    }
    (kept, duplicates)
}

fn action_hotkey(settings: &Settings, name: &str) -> Option<Hotkey> {
    HOTKEY_ACTIONS
        .contains(&name)
        .then(|| settings.hotkey(name))
}

/// Live global hotkeys.
///
/// [`HotkeyManager::set_hotkeys`] replaces the whole set, returning the outcome per
/// action; [`HotkeyManager::stop`] unbinds everything.
pub struct HotkeyManager {
    backend: Backend,
    handler: Mutex<Option<HotkeyHandler>>,
    status: Mutex<Vec<(String, Registration)>>,
}

impl HotkeyManager {
    /// Start a manager with no hotkeys bound.
    pub fn new() -> Result<HotkeyManager, HotkeyError> {
        Ok(HotkeyManager {
            backend: Backend::new()?,
            handler: Mutex::new(None),
            status: Mutex::new(Vec::new()),
        })
    }

    /// Install the callback invoked on every press. Replaces any previous callback.
    pub fn set_handler<F>(&self, handler: F)
    where
        F: FnMut(&str) + Send + 'static,
    {
        if let Ok(mut slot) = self.handler.lock() {
            *slot = Some(Box::new(handler));
        }
    }

    /// Bind the given actions, replacing whatever was bound before.
    ///
    /// Returns the outcome per action. A failed bind leaves that action unbound but
    /// does not stop the others.
    pub fn set_hotkeys(&self, hotkeys: &[(String, Hotkey)]) -> Vec<(String, Registration)> {
        let _ = self.stop();
        let results = self.backend.bind(hotkeys);
        if let Ok(mut status) = self.status.lock() {
            *status = results.clone();
        }
        results
    }

    /// Bind whatever `settings` configures, marking duplicates as taken.
    pub fn apply_settings(&self, settings: &Settings) -> Vec<(String, Registration)> {
        let (kept, duplicates) = configured_hotkeys(settings);
        let mut results = self.set_hotkeys(&kept);
        for name in duplicates {
            if let Some(entry) = results.iter_mut().find(|(n, _)| *n == name) {
                entry.1 = Registration::Taken;
            } else {
                results.push((name, Registration::Taken));
            }
        }
        if let Ok(mut status) = self.status.lock() {
            *status = results.clone();
        }
        results
    }

    /// Outcome of the most recent bind, for the settings page.
    pub fn status(&self) -> Vec<(String, Registration)> {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Whether every configured action is bound.
    pub fn all_ok(&self) -> bool {
        self.status()
            .iter()
            .all(|(_, r)| matches!(r, Registration::Ok | Registration::Unset))
    }

    /// Unbind everything. Safe to call when nothing is bound.
    pub fn stop(&self) -> Result<(), HotkeyError> {
        self.backend.stop()
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        let _ = self.backend.stop();
    }
}

// ---------------------------------------------------------------- platforms

#[cfg(target_os = "windows")]
mod backend {
    use super::*;
    use std::collections::HashMap;

    /// `MOD_NOREPEAT` keeps a held key from firing on every auto-repeat.
    const MOD_NOREPEAT: u32 = 0x4000;
    const WM_HOTKEY: u32 = 0x0312;
    const WM_QUIT: u32 = 0x0012;

    /// Commands to the hotkey thread.
    enum Cmd {
        Bind(Vec<(String, Hotkey)>, Sender<Vec<(String, Registration)>>),
        Stop(Sender<()>),
    }

    pub struct Backend {
        tx: Option<Sender<Cmd>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Backend {
        pub fn new() -> Result<Backend, HotkeyError> {
            let (tx, rx) = mpsc::channel::<Cmd>();
            let thread = std::thread::Builder::new()
                .name("rewind-hotkeys".into())
                .spawn(move || run_thread(rx))
                .map_err(|e| HotkeyError::Backend(e.to_string()))?;
            Ok(Backend {
                tx: Some(tx),
                thread: Some(thread),
            })
        }

        pub fn bind(&self, hotkeys: &[(String, Hotkey)]) -> Vec<(String, Registration)> {
            let Some(tx) = self.tx.as_ref() else {
                return hotkeys
                    .iter()
                    .map(|(n, _)| (n.clone(), Registration::Taken))
                    .collect();
            };
            let (reply_tx, reply_rx) = mpsc::channel();
            if tx.send(Cmd::Bind(hotkeys.to_vec(), reply_tx)).is_err() {
                return hotkeys
                    .iter()
                    .map(|(n, _)| (n.clone(), Registration::Taken))
                    .collect();
            }
            // Registration itself is immediate; the wait only covers thread wake-up.
            reply_rx
                .recv_timeout(Duration::from_secs(3))
                .unwrap_or_else(|_| {
                    hotkeys
                        .iter()
                        .map(|(n, _)| (n.clone(), Registration::Taken))
                        .collect()
                })
        }

        pub fn stop(&self) -> Result<(), HotkeyError> {
            let Some(tx) = self.tx.as_ref() else {
                return Ok(());
            };
            let (reply_tx, reply_rx) = mpsc::channel();
            if tx.send(Cmd::Stop(reply_tx)).is_err() {
                return Ok(());
            }
            let _ = reply_rx.recv_timeout(Duration::from_millis(1500));
            Ok(())
        }
    }

    impl Drop for Backend {
        fn drop(&mut self) {
            let _ = self.stop();
            self.tx = None;
            if let Some(handle) = self.thread.take() {
                let _ = handle.join();
            }
        }
    }

    fn run_thread(rx: mpsc::Receiver<Cmd>) {
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, HOT_KEY_MODIFIERS};
        use windows::Win32::UI::WindowsAndMessaging::{
            PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE,
        };

        // SAFETY: PeekMessageW here creates this thread's message queue, which
        // RegisterHotKey needs; it touches only this thread's own state.
        unsafe {
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        }

        let mut bound: HashMap<i32, String> = HashMap::new();

        loop {
            // Service commands without blocking the message pump for long.
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Cmd::Bind(hotkeys, reply)) => {
                    unbind_all(&bound);
                    bound.clear();
                    let mut results = Vec::with_capacity(hotkeys.len());
                    for (index, (name, hotkey)) in hotkeys.iter().enumerate() {
                        let id = (index as i32) + 1;
                        if hotkey.vk == 0 {
                            results.push((name.clone(), Registration::Unset));
                            continue;
                        }
                        // SAFETY: registering on this thread, which owns the message queue.
                        let ok = unsafe {
                            RegisterHotKey(
                                None,
                                id,
                                HOT_KEY_MODIFIERS(hotkey.mods | MOD_NOREPEAT),
                                hotkey.vk,
                            )
                            .is_ok()
                        };
                        if ok {
                            bound.insert(id, name.clone());
                            results.push((name.clone(), Registration::Ok));
                        } else {
                            results.push((name.clone(), Registration::Taken));
                        }
                    }
                    let _ = reply.send(results);
                }
                Ok(Cmd::Stop(reply)) => {
                    unbind_all(&bound);
                    bound.clear();
                    let _ = reply.send(());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            // Drain whatever the queue holds without blocking.
            loop {
                use windows::Win32::UI::WindowsAndMessaging::PM_REMOVE;
                let mut msg = MSG::default();
                // SAFETY: reading this thread's queue.
                let has = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() };
                if !has {
                    break;
                }
                if msg.message == WM_HOTKEY {
                    let id = msg.wParam.0 as i32;
                    if let Some(name) = bound.get(&id) {
                        crate::hotkeys::dispatch(name);
                    }
                }
            }
        }

        // SAFETY: posting a quit to our own thread on the way out.
        unsafe {
            let _ = PostThreadMessageW(
                GetCurrentThreadId(),
                WM_QUIT,
                Default::default(),
                Default::default(),
            );
        }
    }

    fn unbind_all(bound: &HashMap<i32, String>) {
        use windows::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey;
        for id in bound.keys() {
            // SAFETY: unregistering ids we registered.
            unsafe {
                let _ = UnregisterHotKey(None, *id);
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod backend {
    use super::*;
    use std::ffi::c_void;

    /// Four-character class/kind identifying a hotkey press event.
    const K_EVENT_CLASS_KEYBOARD: u32 = fourcc("keyb");
    const K_EVENT_HOTKEY_PRESSED: u32 = 5;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct EventHotKeyID {
        signature: u32,
        id: u32,
    }

    #[repr(C)]
    struct EventTypeSpec {
        event_class: u32,
        event_kind: u32,
    }

    type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

    /// `MOD_ALT`/`MOD_CONTROL`/`MOD_SHIFT`/`MOD_WIN` → Carbon modifier masks.
    const CARBON_MODS: [(u32, u32); 4] =
        [(0x1, 0x0800), (0x2, 0x1000), (0x4, 0x0200), (0x8, 0x0100)];

    const fn fourcc(code: &str) -> u32 {
        let b = code.as_bytes();
        ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | (b[3] as u32)
    }

    pub fn carbon_mods(mods: u32) -> u32 {
        CARBON_MODS
            .iter()
            .filter(|(flag, _)| mods & flag != 0)
            .map(|(_, carbon)| *carbon)
            .sum()
    }

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn GetApplicationEventTarget() -> *mut c_void;
        fn InstallEventHandler(
            target: *mut c_void,
            handler: Handler,
            num_specs: usize,
            specs: *const EventTypeSpec,
            user_data: *mut c_void,
            out_ref: *mut *mut c_void,
        ) -> i32;
        fn RegisterEventHotKey(
            key_code: u32,
            modifiers: u32,
            hotkey_id: EventHotKeyID,
            target: *mut c_void,
            options: u32,
            out_ref: *mut *mut c_void,
        ) -> i32;
        fn UnregisterEventHotKey(ref_: *mut c_void) -> i32;
        fn GetEventParameter(
            event: *mut c_void,
            name: u32,
            desired_type: u32,
            actual_type: *mut u32,
            buffer_size: usize,
            out_size: *mut usize,
            data: *mut c_void,
        ) -> i32;
    }

    pub struct Backend {
        refs: Mutex<Vec<*mut c_void>>,
    }

    // SAFETY: the raw pointers are opaque Carbon handles only ever used from the main
    // thread, guarded by the mutexes above.
    unsafe impl Send for Backend {}
    unsafe impl Sync for Backend {}

    impl Backend {
        pub fn new() -> Result<Backend, HotkeyError> {
            Ok(Backend {
                refs: Mutex::new(Vec::new()),
            })
        }

        /// Install the shared event handler once, then register each combo.
        pub fn bind(&self, hotkeys: &[(String, Hotkey)]) -> Vec<(String, Registration)> {
            if !ensure_handler_installed() {
                return all_taken(hotkeys);
            }
            let mut refs = match self.refs.lock() {
                Ok(g) => g,
                Err(_) => return all_taken(hotkeys),
            };
            let mut results = Vec::with_capacity(hotkeys.len());
            for (index, (name, hotkey)) in hotkeys.iter().enumerate() {
                if hotkey.vk == 0 {
                    results.push((name.clone(), Registration::Unset));
                    continue;
                }
                let mut out: *mut c_void = std::ptr::null_mut();
                let id = EventHotKeyID {
                    signature: fourcc("RWND"),
                    id: (index as u32) + 1,
                };
                // SAFETY: registering with the app target; `out` is only written on success.
                let err = unsafe {
                    RegisterEventHotKey(
                        hotkey.vk,
                        carbon_mods(hotkey.mods),
                        id,
                        GetApplicationEventTarget(),
                        0,
                        &mut out,
                    )
                };
                if err == 0 {
                    register_index((index as u32) + 1, name);
                    refs.push(out);
                    results.push((name.clone(), Registration::Ok));
                } else {
                    results.push((name.clone(), Registration::Taken));
                }
            }
            results
        }

        pub fn stop(&self) -> Result<(), HotkeyError> {
            let Ok(mut refs) = self.refs.lock() else {
                return Ok(());
            };
            for r in refs.drain(..) {
                // SAFETY: refs came from RegisterEventHotKey and are released once.
                unsafe {
                    UnregisterEventHotKey(r);
                }
            }
            clear_names();
            Ok(())
        }
    }

    /// Install the process-wide hotkey handler exactly once.
    ///
    /// The Carbon application event target is shared by the whole process, so every
    /// manager must reuse one handler; installing a second one is rejected by Carbon
    /// and would leave that manager silently unbound.
    fn ensure_handler_installed() -> bool {
        static INSTALLED: Mutex<bool> = Mutex::new(false);
        let Ok(mut installed) = INSTALLED.lock() else {
            return false;
        };
        if *installed {
            return true;
        }
        let spec = EventTypeSpec {
            event_class: K_EVENT_CLASS_KEYBOARD,
            event_kind: K_EVENT_HOTKEY_PRESSED,
        };
        let mut handler_ref: *mut c_void = std::ptr::null_mut();
        // SAFETY: installing on the application event target, once per process.
        let err = unsafe {
            InstallEventHandler(
                GetApplicationEventTarget(),
                on_hotkey,
                1,
                &spec,
                std::ptr::null_mut(),
                &mut handler_ref,
            )
        };
        if err == 0 {
            *installed = true;
            true
        } else {
            false
        }
    }

    /// Post a `KEventHotKeyPressed` for hotkey `id` on the application event target.
    ///
    /// Synthetic key presses never reach `RegisterEventHotKey`, so this is how a press
    /// can be simulated: it goes through the same dispatch path a real one would.
    #[cfg(test)]
    pub fn post_hotkey_press(id: u32) {
        #[link(name = "Carbon", kind = "framework")]
        extern "C" {
            fn CreateEvent(
                allocator: *mut c_void,
                class: u32,
                kind: u32,
                when: f64,
                flags: u32,
                out_event: *mut *mut c_void,
            ) -> i32;
            fn SetEventParameter(
                event: *mut c_void,
                name: u32,
                type_: u32,
                size: usize,
                data: *const c_void,
            ) -> i32;
            fn SendEventToEventTarget(event: *mut c_void, target: *mut c_void) -> i32;
        }
        let mut event: *mut c_void = std::ptr::null_mut();
        // SAFETY: creating and filling our own event, then handing it to Carbon.
        unsafe {
            if CreateEvent(
                std::ptr::null_mut(),
                K_EVENT_CLASS_KEYBOARD,
                K_EVENT_HOTKEY_PRESSED,
                0.0,
                0,
                &mut event,
            ) != 0
            {
                return;
            }
            let hk = EventHotKeyID {
                signature: fourcc("RWND"),
                id,
            };
            if SetEventParameter(
                event,
                fourcc("----"),
                fourcc("hkid"),
                std::mem::size_of::<EventHotKeyID>(),
                &hk as *const _ as *const c_void,
            ) != 0
            {
                return;
            }
            SendEventToEventTarget(event, GetApplicationEventTarget());
        }
    }

    fn all_taken(hotkeys: &[(String, Hotkey)]) -> Vec<(String, Registration)> {
        hotkeys
            .iter()
            .map(|(n, hk)| {
                let r = if hk.vk == 0 {
                    Registration::Unset
                } else {
                    Registration::Taken
                };
                (n.clone(), r)
            })
            .collect()
    }

    extern "C" fn on_hotkey(_call: *mut c_void, event: *mut c_void, _user: *mut c_void) -> i32 {
        let mut id = EventHotKeyID {
            signature: 0,
            id: 0,
        };
        // SAFETY: Carbon fills `id` for a hotkey event we registered.
        let err = unsafe {
            GetEventParameter(
                event,
                fourcc("----"),
                fourcc("hkid"),
                std::ptr::null_mut(),
                std::mem::size_of::<EventHotKeyID>(),
                std::ptr::null_mut(),
                &mut id as *mut _ as *mut c_void,
            )
        };
        if err == 0 {
            if let Some(name) = name_for(id.id) {
                crate::hotkeys::dispatch(&name);
            }
        }
        0
    }

    /// Hotkey id → action name, filled at registration because the callback cannot
    /// safely reach back into the manager.
    fn registry() -> &'static Mutex<std::collections::HashMap<u32, String>> {
        static REGISTRY: std::sync::LazyLock<Mutex<std::collections::HashMap<u32, String>>> =
            std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));
        &REGISTRY
    }

    fn register_index(id: u32, name: &str) {
        if let Ok(mut map) = registry().lock() {
            map.insert(id, name.to_string());
        }
    }

    fn name_for(id: u32) -> Option<String> {
        registry().lock().ok()?.get(&id).cloned()
    }

    fn clear_names() {
        if let Ok(mut map) = registry().lock() {
            map.clear();
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod backend {
    use super::*;

    pub struct Backend;

    impl Backend {
        pub fn new() -> Result<Backend, HotkeyError> {
            Err(HotkeyError::Unsupported(
                "global hotkeys are only implemented on Windows and macOS".into(),
            ))
        }
        pub fn bind(&self, _hotkeys: &[(String, Hotkey)]) -> Vec<(String, Registration)> {
            Vec::new()
        }
        pub fn stop(&self) -> Result<(), HotkeyError> {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------- dispatch

/// How a press reaches the app's handler.
///
/// The platform callbacks are plain C functions with no user-data pointer, and Carbon
/// runs them on the main thread, so the handler is installed into a process-global
/// slot. One manager per process is the supported shape, which matches the app.
pub type HotkeyHandler = Box<dyn FnMut(&str) + Send>;

fn handler_slot() -> &'static Mutex<Option<HotkeyHandler>> {
    static SLOT: std::sync::LazyLock<Mutex<Option<HotkeyHandler>>> =
        std::sync::LazyLock::new(|| Mutex::new(None));
    &SLOT
}

/// Deliver a press to the installed handler.
pub(crate) fn dispatch(action: &str) {
    if let Ok(mut slot) = handler_slot().lock() {
        if let Some(handler) = slot.as_mut() {
            handler(action);
        }
    }
}

/// Install the process-wide handler used by the platform callbacks.
pub fn set_global_handler<F>(handler: F)
where
    F: FnMut(&str) + Send + 'static,
{
    if let Ok(mut slot) = handler_slot().lock() {
        *slot = Some(Box::new(handler));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real hotkey registration is process-global, so tests that touch the OS must not
    /// run together: a combo held by one test makes another report it as taken.
    #[cfg(target_os = "macos")]
    fn hotkey_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn hk(mods: u32, vk: u32) -> Hotkey {
        Hotkey {
            mods,
            vk,
            text: format!("{mods}+{vk}"),
        }
    }

    #[test]
    fn configured_hotkeys_lists_all_five_actions() {
        let settings = Settings::default();
        let (kept, dupes) = configured_hotkeys(&settings);
        assert_eq!(kept.len(), 5, "defaults define five hotkeys");
        assert!(dupes.is_empty());
        let names: Vec<&str> = kept.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, HOTKEY_ACTIONS);
    }

    #[test]
    fn duplicate_combos_are_reported_not_registered_twice() {
        let mut settings = Settings::default();
        let toggle = settings.hotkey("toggle");
        settings.set_hotkey("clip", &toggle);
        let (kept, dupes) = configured_hotkeys(&settings);
        assert_eq!(kept.len(), 4, "the later duplicate is dropped");
        assert_eq!(dupes, vec!["clip".to_string()]);
        assert!(!kept.iter().any(|(n, _)| n == "clip"));
    }

    #[test]
    fn three_actions_may_not_share_one_combo() {
        let mut settings = Settings::default();
        let toggle = settings.hotkey("toggle");
        settings.set_hotkey("record", &toggle);
        settings.set_hotkey("screenshot", &toggle);
        let (kept, dupes) = configured_hotkeys(&settings);
        assert_eq!(kept.len(), 3);
        assert_eq!(dupes, vec!["record".to_string(), "screenshot".to_string()]);
    }

    #[test]
    fn unset_hotkey_is_not_a_duplicate() {
        let mut settings = Settings::default();
        settings.set_hotkey("clip", &hk(0, 0));
        settings.set_hotkey("record", &hk(0, 0));
        let (kept, dupes) = configured_hotkeys(&settings);
        assert_eq!(kept.len(), 5, "unset entries are kept so they report Unset");
        assert!(dupes.is_empty(), "two unset hotkeys are not the same combo");
    }

    #[test]
    fn registrations_report_ok_taken_and_unset_distinctly() {
        assert!(Registration::Ok.is_ok());
        assert!(!Registration::Taken.is_ok());
        assert!(!Registration::Unset.is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn carbon_modifiers_match_the_python_mapping() {
        use crate::hotkeys::backend::carbon_mods;
        // Defaults on macOS are Control + Option: 0x2 | 0x1.
        assert_eq!(carbon_mods(0x3), 0x1000 | 0x0800);
        assert_eq!(carbon_mods(0x1), 0x0800);
        assert_eq!(carbon_mods(0x2), 0x1000);
        assert_eq!(carbon_mods(0x4), 0x0200);
        assert_eq!(carbon_mods(0x8), 0x0100);
        assert_eq!(carbon_mods(0), 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn default_settings_register_on_this_machine() {
        let _guard = hotkey_test_lock();
        // Registration must succeed for the shipped defaults; this is the check that a
        // combo macOS refuses (Option/Shift only) would fail.
        let Ok(manager) = HotkeyManager::new() else {
            eprintln!("skipping: hotkey backend unavailable");
            return;
        };
        let settings = Settings::default();
        let results = manager.apply_settings(&settings);
        assert_eq!(results.len(), 5, "{results:?}");
        for (name, reg) in &results {
            assert_eq!(
                *reg,
                Registration::Ok,
                "{name} should register with the shipped defaults"
            );
        }
        assert!(manager.all_ok());
        manager.stop().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stopping_releases_the_combos_so_they_can_bind_again() {
        let _guard = hotkey_test_lock();
        let Ok(manager) = HotkeyManager::new() else {
            eprintln!("skipping: hotkey backend unavailable");
            return;
        };
        let settings = Settings::default();
        assert!(manager
            .apply_settings(&settings)
            .iter()
            .all(|(_, r)| r.is_ok()));
        manager.stop().unwrap();
        // Re-binding after a stop must work; a leaked registration would report Taken.
        let again = manager.apply_settings(&settings);
        assert!(
            again.iter().all(|(_, r)| r.is_ok()),
            "rebind after stop: {again:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_press_reaches_the_handler_with_its_action_name() {
        let _guard = hotkey_test_lock();
        let Ok(manager) = HotkeyManager::new() else {
            eprintln!("skipping: hotkey backend unavailable");
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        set_global_handler(move |action| {
            let _ = tx.send(action.to_string());
        });
        let results = manager.apply_settings(&Settings::default());
        assert!(results.iter().all(|(_, r)| r.is_ok()), "{results:?}");

        // Hotkey ids are assigned in HOTKEY_ACTIONS order, starting at 1.
        for (index, expected) in HOTKEY_ACTIONS.iter().enumerate() {
            crate::hotkeys::backend::post_hotkey_press(index as u32 + 1);
            let got = rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap_or_else(|_| panic!("{expected} never dispatched"));
            assert_eq!(&got, expected, "hotkey id {} mapped to {got}", index + 1);
        }
        manager.stop().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_press_after_stop_is_not_dispatched() {
        let _guard = hotkey_test_lock();
        let Ok(manager) = HotkeyManager::new() else {
            eprintln!("skipping: hotkey backend unavailable");
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        set_global_handler(move |action| {
            let _ = tx.send(action.to_string());
        });
        assert!(manager
            .apply_settings(&Settings::default())
            .iter()
            .all(|(_, r)| r.is_ok()));
        crate::hotkeys::backend::post_hotkey_press(1);
        assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).is_ok());

        manager.stop().unwrap();
        // The name mapping is cleared on stop, so a stale press must not fire.
        crate::hotkeys::backend::post_hotkey_press(1);
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(400))
                .is_err(),
            "a hotkey press fired after stop()"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_owned_combo_is_reported_as_taken() {
        let _guard = hotkey_test_lock();
        let Ok(first) = HotkeyManager::new() else {
            eprintln!("skipping: hotkey backend unavailable");
            return;
        };
        let Ok(second) = HotkeyManager::new() else {
            eprintln!("skipping: hotkey backend unavailable");
            return;
        };
        let settings = Settings::default();
        assert!(first
            .apply_settings(&settings)
            .iter()
            .all(|(_, r)| r.is_ok()));
        // A second manager asking for the same combos must be told they are taken.
        let results = second.apply_settings(&settings);
        assert!(
            results.iter().any(|(_, r)| *r == Registration::Taken),
            "expected collisions, got {results:?}"
        );
        first.stop().unwrap();
    }
}
