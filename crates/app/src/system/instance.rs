//! Single instance: a second launch pings the running AgentDeck, which shows its window, then exits.

use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;
use std::time::Duration;

use gpui_kit::*;

/// Port held for the app lifetime: a second instance fails to bind.
/// shortcut: a TCP port as the instance lock; switch to a named mutex if the port ever collides.
const INSTANCE_PORT: u16 = 47799;

static LISTENER: OnceLock<TcpListener> = OnceLock::new();

/// Returns the lock to keep alive for the whole run, or None when AgentDeck already runs
/// (the running instance is asked to come to the front).
pub fn single_instance() -> Option<TcpListener> {
    match TcpListener::bind(("127.0.0.1", INSTANCE_PORT)) {
        Ok(listener) => {
            if let Ok(clone) = listener.try_clone() {
                let _ = LISTENER.set(clone);
            }
            Some(listener)
        }
        Err(_) => {
            let addr = ([127, 0, 0, 1], INSTANCE_PORT).into();
            if let Err(e) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) {
                eprintln!("cannot reach the running AgentDeck: {e}");
            }
            None
        }
    }
}

/// Every connection to the instance port means "another launch": show the main window.
pub(super) fn listen(cx: &mut App) {
    let Some(listener) = LISTENER.get() else {
        return;
    };
    let Ok(listener) = listener.try_clone() else {
        return;
    };
    let (tx, rx) = smol::channel::unbounded::<()>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if stream.is_ok() && tx.send_blocking(()).is_err() {
                break;
            }
        }
    });
    cx.spawn(async move |cx| {
        while rx.recv().await.is_ok() {
            cx.update(super::show_main);
        }
    })
    .detach();
}
