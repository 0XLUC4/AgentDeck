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
/// then written after it, minus the leading chunks the replay already contains.
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

    /// The scrollback: returns what to write, in order. A second replay is ignored.
    pub fn replay(&mut self, text: &str) -> Vec<String> {
        let Some(pending) = self.pending.take() else {
            return Vec::new();
        };
        // Overlap is detected by suffix match, so a repeated chunk (spinner frame) may be dropped once.
        let mut skip = pending.len();
        while skip > 0 && !text.ends_with(&pending[..skip].concat()) {
            skip -= 1;
        }
        let mut out = Vec::new();
        if !text.is_empty() {
            out.push(text.to_owned());
        }
        out.extend(pending.into_iter().skip(skip));
        out
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
        let mut g = ReplayGate::new();
        assert!(g.output("c").is_none());
        assert_eq!(g.replay("ab").concat(), "abc");
        assert_eq!(g.output("d").as_deref(), Some("d"));

        let mut g = ReplayGate::new();
        g.output("c");
        g.output("d");
        assert_eq!(g.replay("abc").concat(), "abcd");

        let mut g = ReplayGate::new();
        g.output("x");
        assert_eq!(g.replay(""), ["x"]);
        assert!(g.replay("late").is_empty());
    }

    #[test]
    fn one_window_owns_input() {
        assert!(forwards_input(false, false));
        assert!(!forwards_input(true, false));
        assert!(!forwards_input(false, true));
        assert!(forwards_input(true, true));
    }
}
