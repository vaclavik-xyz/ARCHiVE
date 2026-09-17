//! Lightweight in-process progress reporting for the `recover` pipeline.
//!
//! `run_recover` announces the total number of expected sections up front
//! (counted from the manifest), then reports the current section as it works;
//! the per-asset extract loops update a 0..1 fraction (photos is the long pole),
//! and the `ui` server reads the snapshot in `/api/status` so the wizard can
//! show a determinate progress bar with percent + ETA. Deliberately global +
//! Mutex-guarded: single recovery at a time by design.

use std::sync::Mutex;

/// Current progress snapshot.
#[derive(Debug, Clone)]
pub struct Progress {
    /// Human label of the section being processed right now (e.g. "Fotky a videa").
    pub current: String,
    /// Completed sections so far.
    pub done: usize,
    /// Total sections expected (present stores counted from the manifest).
    pub total: usize,
    /// Optional 0..1 progress within the current section (per-asset loops).
    pub fraction: f32,
    /// Optional fine-grained detail text (e.g. "512/1635").
    pub detail: String,
}

impl Progress {
    /// Overall completion 0..100 — sections carry equal weight, the current
    /// section contributes its asset fraction.
    pub fn percent(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        let done = self.done as f32 + self.fraction.clamp(0.0, 1.0);
        ((done / self.total as f32) * 100.0).clamp(0.0, 100.0)
    }
}

static PROGRESS: Mutex<Option<Progress>> = Mutex::new(None);

/// Announce the total expected sections and reset counters (recovery start).
pub fn begin(total: usize) {
    *PROGRESS.lock().unwrap() = Some(Progress {
        current: "Otevírám zálohu".into(),
        done: 0,
        total,
        fraction: 0.0,
        detail: String::new(),
    });
}

/// Mark the start of a section: shows its label and clears the fraction.
pub fn set(current: &str, done: usize, total: usize) {
    let mut slot = PROGRESS.lock().unwrap();
    let entry = slot.get_or_insert_with(|| Progress {
        current: String::new(),
        done: 0,
        total,
        fraction: 0.0,
        detail: String::new(),
    });
    entry.current = current.to_string();
    entry.done = done;
    entry.total = entry.total.max(total);
    entry.fraction = 0.0;
    entry.detail.clear();
}

/// Update the 0..1 fraction of the current section (per-asset loops).
pub fn fraction(value: f32) {
    let mut slot = PROGRESS.lock().unwrap();
    if let Some(entry) = slot.as_mut() {
        entry.fraction = value.clamp(0.0, 1.0);
    }
}

/// Update the fine-grained detail text of the current section.
pub fn detail(text: &str) {
    let mut slot = PROGRESS.lock().unwrap();
    if let Some(entry) = slot.as_mut() {
        entry.detail = text.to_string();
    }
}

/// A section finished — bump the completed counter and show it as current.
/// The total estimate grows to cover `done` if the upfront count was low.
pub fn section_done(current: &str, done: usize, total: usize) {
    let mut slot = PROGRESS.lock().unwrap();
    let entry = slot.get_or_insert_with(|| Progress {
        current: String::new(),
        done: 0,
        total,
        fraction: 0.0,
        detail: String::new(),
    });
    entry.current = current.to_string();
    entry.done = done;
    entry.total = entry.total.max(total).max(done);
    entry.fraction = 0.0;
    entry.detail.clear();
}

/// The recovery finished — normalize the snapshot to the actual section count
/// so the percent reaches exactly 100 even when the upfront estimate overcounted
/// (a manifest-present store whose loader returned nothing).
pub fn finish(actual_sections: usize) {
    let mut slot = PROGRESS.lock().unwrap();
    if let Some(entry) = slot.as_mut() {
        entry.done = actual_sections.max(entry.done);
        entry.total = entry.done;
        entry.fraction = 0.0;
        entry.detail.clear();
    }
}

/// Read the snapshot, if a recovery is reporting progress.
pub fn get() -> Option<Progress> {
    PROGRESS.lock().unwrap().clone()
}

/// Reset (called when a new recovery starts).
pub fn reset() {
    *PROGRESS.lock().unwrap() = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snapshot is a process-global; cargo runs tests concurrently, so the
    /// tests must hold this lock for their whole body or they race on it.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn set_get_and_reset_roundtrip() {
        let _g = TEST_LOCK.lock().unwrap();
        reset();
        assert!(get().is_none());
        begin(10);
        set("Fotky a videa", 2, 10);
        fraction(0.5);
        detail("512/1635");
        let p = get().unwrap();
        assert_eq!(p.current, "Fotky a videa");
        assert_eq!(p.done, 2);
        assert_eq!(p.total, 10);
        assert_eq!(p.detail, "512/1635");
        assert!((p.percent() - 25.0).abs() < 0.01); // (2 + 0.5) / 10
        section_done("WhatsApp", 3, 10);
        let p = get().unwrap();
        assert_eq!(p.current, "WhatsApp");
        assert_eq!(p.done, 3);
        assert_eq!(p.fraction, 0.0);
        assert_eq!(p.detail, "");
        reset();
        assert!(get().is_none());
    }

    #[test]
    fn total_grows_if_sections_exceed_estimate() {
        let _g = TEST_LOCK.lock().unwrap();
        reset();
        begin(2);
        set("A", 1, 2);
        section_done("B", 2, 2);
        section_done("C", 3, 2); // estimate was low — total bumps up
        let p = get().unwrap();
        assert_eq!(p.done, 3);
        assert_eq!(p.total, 3);
        assert!((p.percent() - 100.0).abs() < 0.01);
        reset();
    }

    #[test]
    fn percent_clamps() {
        let _g = TEST_LOCK.lock().unwrap();
        reset();
        begin(1);
        set("A", 0, 1);
        fraction(2.0); // out of range clamps to 1
        assert!((get().unwrap().percent() - 100.0).abs() < 0.01);
        reset();
    }

    #[test]
    fn finish_normalizes_an_overcounted_estimate_to_100_percent() {
        let _g = TEST_LOCK.lock().unwrap();
        reset();
        // Upfront estimate counted 5 stores, but only 3 produced sections
        // (present-but-empty stores are skipped by their loaders).
        begin(5);
        set("A", 0, 5);
        section_done("B", 1, 5);
        section_done("C", 2, 5);
        section_done("D", 3, 5);
        let p = get().unwrap();
        assert!(p.percent() < 100.0); // stuck below 100 while running — real
        finish(3); // run completed with 3 actual sections
        let p = get().unwrap();
        assert_eq!(p.done, 3);
        assert_eq!(p.total, 3);
        assert!((p.percent() - 100.0).abs() < 0.01);
        reset();
    }

    #[test]
    fn finish_without_progress_is_a_no_op() {
        let _g = TEST_LOCK.lock().unwrap();
        reset();
        finish(3);
        assert!(get().is_none());
    }
}
