//! Layout helpers: auto grid (`src/lib/gridLayout.ts`) and detached pane windows (`src/lib/detach.ts`).

/// Number of panes in each row, top to bottom: cols = ceil(sqrt(n)). 3 -> [2, 1], 5 -> [3, 2].
pub fn grid_rows(count: usize) -> Vec<usize> {
    if count == 0 {
        return Vec::new();
    }
    let cols = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(cols);
    (0..rows).map(|i| cols.min(count - i * cols)).collect()
}

/// Splits `total` px into `parts` integer sizes that sum exactly to `total`.
pub fn split_sizes(total: u32, parts: usize) -> Vec<u32> {
    if parts == 0 {
        return Vec::new();
    }
    let base = total / parts as u32;
    (0..parts)
        .map(|i| if i == parts - 1 { total - base * (parts as u32 - 1) } else { base })
        .collect()
}

/// Exactly one terminal per pane forwards input and resizes (main window or its detached window).
pub fn forwards_input(in_detached_window: bool, pane_detached: bool) -> bool {
    in_detached_window == pane_detached
}

/// Orders scrollback replay before live output: live chunks received before the replay are held,
/// then written after the part of the replay they are not already in.
#[derive(Debug, Default)]
pub struct ReplayGate {
    pending: Option<Vec<String>>,
}

impl ReplayGate {
    pub fn new() -> Self {
        Self {
            pending: Some(Vec::new()),
        }
    }

    /// Live output: returned to write now, or held until the replay.
    pub fn output(&mut self, data: &str) -> Option<String> {
        match self.pending.as_mut() {
            Some(pending) => {
                pending.push(data.to_owned());
                None
            }
            None => Some(data.to_owned()),
        }
    }

    /// The scrollback: returns the output older than this gate (its terminal queries were asked
    /// before the view existed) and the chunks the gate held, which are new to it and must be
    /// answered (ConPTY's startup `ESC[6n` blocks the agent until it is). A second replay is ignored.
    pub fn replay(&mut self, text: &str) -> Option<(String, Vec<String>)> {
        let pending = self.pending.take()?;
        // Overlap is detected by suffix match, so a repeated chunk (spinner frame) may be dropped once.
        let mut overlap = pending.len();
        while overlap > 0 && !text.ends_with(&pending[..overlap].concat()) {
            overlap -= 1;
        }
        let old = &text[..text.len() - pending[..overlap].concat().len()];
        Some((old.to_owned(), pending))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid() {
        let cases: [(usize, &[usize]); 10] = [
            (1, &[1]),
            (2, &[2]),
            (3, &[2, 1]),
            (4, &[2, 2]),
            (5, &[3, 2]),
            (6, &[3, 3]),
            (7, &[3, 3, 1]),
            (8, &[3, 3, 2]),
            (9, &[3, 3, 3]),
            (10, &[4, 4, 2]),
        ];
        for (n, rows) in cases {
            assert_eq!(grid_rows(n), rows, "{n} panes");
        }
        assert!(grid_rows(0).is_empty());
        assert_eq!(split_sizes(703, 3).iter().sum::<u32>(), 703);
    }

    #[test]
    fn replay_gate() {
        let own = |old: &str, new: &[&str]| Some((old.to_owned(), new.iter().map(|s| s.to_string()).collect()));
        let mut g = ReplayGate::new();
        assert!(g.output("c").is_none());
        assert_eq!(g.replay("ab"), own("ab", &["c"]));
        assert_eq!(g.output("d").as_deref(), Some("d"));

        // Held chunks already in the scrollback come back as new, not as old replay.
        let mut g = ReplayGate::new();
        g.output("c");
        g.output("d");
        assert_eq!(g.replay("abc"), own("ab", &["c", "d"]));

        let mut g = ReplayGate::new();
        g.output("\x1b[6n");
        assert_eq!(g.replay("\x1b[6n"), own("", &["\x1b[6n"]));
        assert!(g.replay("late").is_none());
    }

    #[test]
    fn one_window_owns_input() {
        assert!(forwards_input(false, false));
        assert!(!forwards_input(true, false));
        assert!(!forwards_input(false, true));
        assert!(forwards_input(true, true));
    }
}
