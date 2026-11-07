//! Global push-to-talk shortcut: registered here, forwarded to `agentdeck_core::voice::on_shortcut`.

use std::sync::mpsc;

use agentdeck_core::voice::{self, Shortcut};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui_kit::*;

use crate::state::Deck;

/// The OS registration (must live on the UI thread, which pumps its messages) and the current key.
struct Hotkey {
    manager: GlobalHotKeyManager,
    current: Option<Shortcut>,
}

impl Global for Hotkey {}

pub(super) fn init(deck: &Entity<Deck>, cx: &mut App) {
    let backend = deck.read(cx).backend.clone();
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("voice: global shortcuts unavailable: {e}");
            return;
        }
    };
    // Another app may own the shortcut: keep the app usable, voice_start still works.
    let current = match voice::voice_get_config(&backend).map(|v| v.config.shortcut) {
        Ok(text) => match text.parse::<Shortcut>() {
            Ok(key) => match manager.register(key) {
                Ok(()) => Some(key),
                Err(e) => {
                    eprintln!("voice: cannot register shortcut {text}: {e}");
                    None
                }
            },
            Err(e) => {
                eprintln!("voice: bad shortcut {text}: {e}");
                None
            }
        },
        Err(e) => {
            eprintln!("voice: config unavailable: {e}");
            None
        }
    };
    let wanted = voice::voice_get_config(&backend).ok().and_then(|v| v.config.shortcut.parse::<Shortcut>().ok());
    cx.set_global(Hotkey { manager, current });
    // Usually another app (the old AgentDeck) holds the key: say so (a release build has no console)
    // and keep retrying, so voice works as soon as that app lets it go.
    if current.is_none()
        && let Some(key) = wanted
    {
        deck.update(cx, |d, cx| {
            d.toast("Voice shortcut is taken by another app (close the old AgentDeck). Retrying...", cx)
        });
        let deck = deck.downgrade();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(5)).await;
                let done = cx.update(|cx| {
                    let hotkey = cx.global_mut::<Hotkey>();
                    if hotkey.current.is_some() {
                        return true; // set from Settings meanwhile
                    }
                    if hotkey.manager.register(key).is_err() {
                        return false;
                    }
                    hotkey.current = Some(key);
                    if let Some(deck) = deck.upgrade() {
                        deck.update(cx, |d, cx| d.toast("Voice shortcut ready", cx));
                    }
                    true
                });
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    // One worker thread keeps press / release in order and off the UI thread (mic start takes a moment).
    let (tx, rx) = mpsc::channel::<bool>();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        let _ = tx.send(event.state() == HotKeyState::Pressed);
    }));
    std::thread::spawn(move || {
        while let Ok(pressed) = rx.recv() {
            voice::on_shortcut(&backend, pressed);
        }
    });
}

/// Swaps the voice shortcut ("Ctrl+Shift+Space"). Call before `voice::voice_set_config`; on error
/// the old shortcut stays registered and the config must not be saved.
pub fn set_voice_shortcut(shortcut: &str, cx: &mut App) -> Result<(), String> {
    let next = shortcut
        .parse::<Shortcut>()
        .map_err(|e| format!("invalid shortcut \"{shortcut}\": {e}"))?;
    let Some(hotkey) = cx.try_global::<Hotkey>() else {
        return Err("global shortcuts are unavailable".into());
    };
    let old = hotkey.current;
    if old == Some(next) {
        return Ok(());
    }
    if let Some(old) = old {
        let _ = hotkey.manager.unregister(old); // may never have been registered
    }
    if let Err(e) = hotkey.manager.register(next) {
        if let Some(old) = old
            && let Err(e) = hotkey.manager.register(old)
        {
            eprintln!("voice: cannot restore shortcut: {e}");
        }
        return Err(format!("cannot register shortcut \"{shortcut}\": {e}"));
    }
    cx.global_mut::<Hotkey>().current = Some(next);
    Ok(())
}
