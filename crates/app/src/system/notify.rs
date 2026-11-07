//! Desktop notification (Windows toast), short two-tone beep and phone push (ntfy topic or webhook).

use agentdeck_core::logic::settings::Settings;

/// Notifies per the user's settings: phone push when `phone_url` is set, beep when `notify.sound`,
/// toast when `notify.enabled`. Never blocks the caller; failures are logged.
pub fn notify(settings: &Settings, title: &str, body: &str) {
    let phone_url = settings.phone_url.clone();
    if !phone_url.is_empty() {
        let (title, body) = (title.to_owned(), body.to_owned());
        std::thread::spawn(move || {
            if let Err(e) = agentdeck_core::notify::push_notify(phone_url, title, body) {
                eprintln!("phone push failed: {e}");
            }
        });
    }
    if settings.notify.sound {
        std::thread::spawn(beep);
    }
    if settings.notify.enabled {
        let (title, body) = (title.to_owned(), body.to_owned());
        std::thread::spawn(move || {
            if let Err(e) = notify_rust::Notification::new()
                .summary(&title)
                .body(&body)
                .appname("AgentDeck")
                .show()
            {
                eprintln!("notification failed: {e}");
            }
        });
    }
}

/// 880 Hz then 660 Hz, like the original WebAudio beep.
#[cfg(windows)]
fn beep() {
    use windows_sys::Win32::System::Diagnostics::Debug::Beep;
    // SAFETY: plain blocking Win32 call with value arguments.
    unsafe {
        Beep(880, 90);
        Beep(660, 160);
    }
}

#[cfg(not(windows))]
fn beep() {}
