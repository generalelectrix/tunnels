//! In-GUI logging capture and rolled-up status indicator.
//!
//! Captures `log` records on a lock-free, non-blocking producer path (the show
//! loop and other real-time threads log here, so it must never block or lock),
//! keeps a finite per-severity scrollback, and derives a sticky-until-viewed
//! alert that wakes an idle GUI the instant a warn/error occurs.
//!
//! Three roles, two boundaries:
//! - Producer (`CaptureLogger`) → drain thread: a bounded `std::sync::mpsc`
//!   channel; `try_send` is lock-free and drops the record when full rather than
//!   blocking a real-time thread.
//! - Drain thread → GUI: a `NotifiedAtomic<AlertCounts>` whose `store` fires the
//!   `RepaintSignal` with no per-update heap allocation.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use chrono::{DateTime, Local};
use eframe::egui::{self, RichText};
use log::{Level, LevelFilter};
use tunnels_lib::notified::{AtomicValue, NotifiedAtomic};
use tunnels_lib::repaint::RepaintSignal;

use crate::STATUS_COLORS;

/// One captured log line. `timestamp` is the local wall-clock time when the
/// producer observed the record, rendered as-is in the GUI (no per-frame work).
pub struct LogRecord {
    pub level: Level,
    pub target: String,
    pub message: String,
    pub timestamp: DateTime<Local>,
}

/// Monotonic per-severity counts of captured records, as the GUI consumes them.
/// Comparing a snapshot against a remembered one yields the sticky alert state.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct AlertTally {
    pub errors: u32,
    pub warns: u32,
}

/// Atomic cell holding an `AlertTally` packed as `(errors << 32) | warns` in a
/// single `AtomicU64`. The packing is private; callers see only `AlertTally`.
pub struct AlertCounts(AtomicU64);

impl AtomicValue for AlertCounts {
    type Value = AlertTally;

    fn new(value: AlertTally) -> Self {
        AlertCounts(AtomicU64::new(pack_tally(value)))
    }

    fn load(&self) -> AlertTally {
        unpack_tally(self.0.load(Ordering::Relaxed))
    }

    fn store(&self, value: AlertTally) {
        self.0.store(pack_tally(value), Ordering::Relaxed);
    }
}

fn pack_tally(tally: AlertTally) -> u64 {
    ((tally.errors as u64) << 32) | (tally.warns as u64)
}

fn unpack_tally(bits: u64) -> AlertTally {
    AlertTally {
        errors: (bits >> 32) as u32,
        warns: bits as u32,
    }
}

/// The shared alert surface written by the drain thread and read by the GUI.
/// The monotonic per-severity `counts` are the only thing that wakes the GUI
/// (a store fires the `RepaintSignal`).
pub struct LogAlert {
    counts: NotifiedAtomic<AlertCounts>,
}

impl LogAlert {
    pub fn new(repaint: RepaintSignal) -> Self {
        Self {
            counts: NotifiedAtomic::new(AlertTally::default(), repaint),
        }
    }

    /// Current per-severity tally. Reading this never wakes the GUI.
    pub fn tally(&self) -> AlertTally {
        self.counts.load()
    }
}

/// Producer-side `log` sink, installed as the global logger in `main()` before
/// the GUI exists (the in-GUI Status view is the only log destination — there is
/// no stderr/terminal output). Captured records are pushed to the drain thread
/// over a bounded channel; the path never blocks or locks.
pub struct CaptureLogger {
    tx: SyncSender<LogRecord>,
}

impl log::Log for CaptureLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        // The global `log::max_level` (driven by the GUI "Capture" dropdown) is
        // the sole gate; this sink captures whatever it's handed.
        true
    }

    fn log(&self, record: &log::Record) {
        let captured = LogRecord {
            level: record.level(),
            target: record.target().to_string(),
            message: record.args().to_string(),
            timestamp: Local::now(),
        };
        // Lock-free and non-blocking: if the drain thread is behind and the
        // channel is full, drop the record rather than block a real-time thread.
        // With a deep channel and a trivial drain step this effectively never
        // happens outside a pathological log storm.
        let _ = self.tx.try_send(captured);
    }

    fn flush(&self) {}
}

/// Build a bounded capture channel. `capacity` bounds the in-flight backlog.
/// The returned `Receiver` is handed to `spawn_drain_thread`.
pub fn channel(capacity: usize) -> (CaptureLogger, Receiver<LogRecord>) {
    let (tx, rx) = sync_channel(capacity);
    (CaptureLogger { tx }, rx)
}

/// Number of `log::Level` variants (Error..Trace), used to size the scrollback.
const NUM_LEVELS: usize = 5;

/// Finite per-severity ring of captured records. Each severity has its own
/// bounded `VecDeque`, so a flood of one severity (e.g. warnings) can never
/// evict records of another (e.g. errors). Indexed by `level as usize - 1`
/// (`Error` = 1 → 0 … `Trace` = 5 → 4), always in range.
pub struct Scrollback {
    by_level: [VecDeque<LogRecord>; NUM_LEVELS],
    cap: usize,
}

impl Scrollback {
    pub fn new(per_severity_cap: usize) -> Self {
        Self {
            by_level: std::array::from_fn(|_| VecDeque::new()),
            cap: per_severity_cap,
        }
    }

    /// Append a record to its severity ring, evicting the oldest entry of that
    /// same severity if the ring is at capacity.
    pub fn push(&mut self, record: LogRecord) {
        let idx = (record.level as usize)
            .saturating_sub(1)
            .min(NUM_LEVELS - 1);
        let deque = &mut self.by_level[idx];
        if self.cap == 0 {
            return;
        }
        if deque.len() >= self.cap {
            deque.pop_front();
        }
        deque.push_back(record);
    }

    /// Records of one severity, oldest first.
    pub fn level(&self, level: Level) -> impl Iterator<Item = &LogRecord> {
        let idx = (level as usize).saturating_sub(1).min(NUM_LEVELS - 1);
        self.by_level[idx].iter()
    }

    /// All retained records at or above `min_level`, merged across the
    /// per-severity rings into a single chronological stream. Each ring is
    /// already time-ordered (records are appended in arrival order), so this is a
    /// lazy k-way merge — a heap of at most one head per ring, no full-list
    /// allocation or per-render sort. Older records of a rare severity interleave
    /// correctly with newer frequent ones.
    pub fn ordered(&self, min_level: LevelFilter) -> impl Iterator<Item = &LogRecord> {
        itertools::kmerge_by(
            self.by_level.iter().map(VecDeque::iter),
            |a: &&LogRecord, b: &&LogRecord| a.timestamp <= b.timestamp,
        )
        .filter(move |record| record.level.to_level_filter() <= min_level)
    }
}

/// Spawn the daemon drain thread. It parks on `rx.recv()` and is woken by each
/// send; on wake it appends every available record to `scrollback`, updates the
/// tally, then publishes the tally through `alert.counts` (which fires the
/// `RepaintSignal`) per the wake policy: always on a new warn/error, and
/// additionally on plain records while `viewing`.
pub fn spawn_drain_thread(
    rx: Receiver<LogRecord>,
    scrollback: Arc<Mutex<Scrollback>>,
    alert: Arc<LogAlert>,
    viewing: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        // `recv` parks until a record arrives; `Err` means all senders are gone
        // (the producer/app shut down), so the thread exits.
        while let Ok(first) = rx.recv() {
            let old = alert.tally();
            let mut tally = old;

            {
                // The GUI is not real-time, so locking here is fine. Append
                // before the publish below so a woken GUI sees the records.
                let mut scrollback = scrollback.lock().unwrap();
                process_record(first, &mut tally, &mut scrollback);
                while let Ok(record) = rx.try_recv() {
                    process_record(record, &mut tally, &mut scrollback);
                }
            }

            let changed = tally != old;
            if changed || viewing.load(Ordering::Relaxed) {
                alert.counts.store(tally);
            }
        }
    })
}

/// Push one record into scrollback and fold its severity into the running tally.
/// Pure bookkeeping; no panics (saturating counts).
fn process_record(record: LogRecord, tally: &mut AlertTally, scrollback: &mut Scrollback) {
    match record.level {
        Level::Error => tally.errors = tally.errors.saturating_add(1),
        Level::Warn => tally.warns = tally.warns.saturating_add(1),
        _ => {}
    }
    scrollback.push(record);
}

/// Rolled-up alert state derived from the live tally versus the last viewed one.
pub enum Alert {
    Calm,
    Warn,
    Error,
}

/// Panel state owned by the App. Holds the shared alert/scrollback handles plus
/// the private `acked` snapshot that implements sticky-until-viewed, the display
/// `min_level` filter (the "Show" combo), and the `capture_level` mirror of the
/// global log floor (the "Capture" combo). Both default to `Warn`.
pub struct LogStatusState {
    alert: Arc<LogAlert>,
    scrollback: Arc<Mutex<Scrollback>>,
    viewing: Arc<AtomicBool>,
    acked: AlertTally,
    min_level: LevelFilter,
    capture_level: LevelFilter,
}

impl LogStatusState {
    /// Construct from the same handles the drain thread received. `acked` starts
    /// at the current tally so a fresh state opens Calm. `capture_level` mirrors
    /// the global log floor set by `main` at startup (`Warn`).
    pub fn new(
        alert: Arc<LogAlert>,
        scrollback: Arc<Mutex<Scrollback>>,
        viewing: Arc<AtomicBool>,
    ) -> Self {
        let acked = alert.tally();
        Self {
            alert,
            scrollback,
            viewing,
            acked,
            min_level: LevelFilter::Warn,
            capture_level: LevelFilter::Warn,
        }
    }

    /// Report whether the Status tab is currently in view. Drives the drain
    /// thread's wake-on-info behavior.
    pub fn set_viewing(&self, active: bool) {
        self.viewing.store(active, Ordering::Relaxed);
    }

    /// Current rolled-up alert: `Error` if new errors since last viewed, else
    /// `Warn` if new warnings, else `Calm`.
    pub fn alert(&self) -> Alert {
        let tally = self.alert.tally();
        if tally.errors > self.acked.errors {
            Alert::Error
        } else if tally.warns > self.acked.warns {
            Alert::Warn
        } else {
            Alert::Calm
        }
    }

    /// Snapshot the current tally as acknowledged, clearing the alert. Called
    /// when the Status panel is rendered (the tab is in view).
    fn mark_viewed(&mut self) {
        self.acked = self.alert.tally();
    }
}

/// Severity levels offered by the Show and Capture dropdowns, most severe first.
/// Trace is intentionally omitted — this project never logs at that level.
const FILTER_LEVELS: [LevelFilter; 4] = [
    LevelFilter::Error,
    LevelFilter::Warn,
    LevelFilter::Info,
    LevelFilter::Debug,
];

/// A level's combo label, colored to match its severity: Error red, Warn amber,
/// the rest the default text color.
fn level_label(level: LevelFilter) -> RichText {
    let label = RichText::new(format!("{level}"));
    match level {
        LevelFilter::Error => label.color(STATUS_COLORS.error),
        LevelFilter::Warn => label.color(STATUS_COLORS.warning),
        _ => label,
    }
}

/// Render wrapper for the Status panel. Rendering marks the alert viewed.
pub struct LogStatusPanel<'a> {
    pub state: &'a mut LogStatusState,
}

impl LogStatusPanel<'_> {
    pub fn ui(self, ui: &mut egui::Ui) {
        // Opening the tab clears the sticky alert.
        self.state.mark_viewed();

        ui.horizontal(|ui| {
            // "Show" filters what the panel displays from what's already captured.
            ui.label("Show:");
            egui::ComboBox::from_id_salt("log_status_min_level")
                .selected_text(level_label(self.state.min_level))
                .show_ui(ui, |ui| {
                    for level in FILTER_LEVELS {
                        ui.selectable_value(&mut self.state.min_level, level, level_label(level));
                    }
                });

            // "Capture" is the global log floor: changing it sets `log::max_level`
            // directly, so more (or fewer) records are emitted app-wide.
            ui.label("Capture:");
            let previous_capture = self.state.capture_level;
            egui::ComboBox::from_id_salt("log_status_capture_level")
                .selected_text(level_label(self.state.capture_level))
                .show_ui(ui, |ui| {
                    for level in FILTER_LEVELS {
                        ui.selectable_value(
                            &mut self.state.capture_level,
                            level,
                            level_label(level),
                        );
                    }
                });
            if self.state.capture_level != previous_capture {
                log::set_max_level(self.state.capture_level);
            }
        });

        ui.separator();

        let scrollback = self.state.scrollback.lock().unwrap();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for record in scrollback.ordered(self.state.min_level) {
                let color = match record.level {
                    Level::Error => STATUS_COLORS.error,
                    Level::Warn => STATUS_COLORS.warning,
                    _ => ui.style().visuals.text_color(),
                };
                ui.label(
                    RichText::new(format!(
                        "{} [{}] {}: {}",
                        record.timestamp.format("%H:%M:%S"),
                        record.level,
                        record.target,
                        record.message
                    ))
                    .color(color),
                );
            }
        });
    }
}

/// Tab-bar label for every page: a colored "⏺ Status" when an unviewed
/// warn/error is pending (error → `STATUS_COLORS.error`, warn →
/// `STATUS_COLORS.warning`), or a plain "Status" when calm.
pub fn status_tab_label(state: &LogStatusState) -> RichText {
    match state.alert() {
        Alert::Error => RichText::new("⏺ Status").color(STATUS_COLORS.error),
        Alert::Warn => RichText::new("⏺ Status").color(STATUS_COLORS.warning),
        Alert::Calm => RichText::new("Status"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use chrono::TimeZone;
    use tunnels_lib::repaint::noop_repaint;

    fn counting_repaint() -> (RepaintSignal, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let count_for_signal = count.clone();
        let signal: RepaintSignal = Arc::new(move || {
            count_for_signal.fetch_add(1, Ordering::Relaxed);
        });
        (signal, count)
    }

    // `log::Record` borrows its `format_args`, so it cannot outlive a helper
    // that builds it. Build inline and hand the record to a callback instead.
    fn emit(logger: &dyn log::Log, level: Level, message: &str) {
        logger.log(
            &log::Record::builder()
                .level(level)
                .target("test")
                .args(format_args!("{message}"))
                .build(),
        );
    }

    fn rec(level: Level, message: &str, ts: DateTime<Local>) -> LogRecord {
        LogRecord {
            level,
            target: "t".into(),
            message: message.into(),
            timestamp: ts,
        }
    }

    #[test]
    fn capture_logger_drops_on_full_without_blocking() {
        let (logger, rx) = channel(2);

        // Five records into a depth-2 channel that nobody is draining. `try_send`
        // is non-blocking, so this returns (rather than hanging): the first two
        // land and the rest are silently dropped.
        for i in 0..5 {
            emit(&logger, Level::Info, &format!("msg {i}"));
        }

        let first = rx.try_recv().expect("first record present");
        let second = rx.try_recv().expect("second record present");
        assert_eq!(first.message, "msg 0");
        assert_eq!(second.message, "msg 1");
        assert!(rx.try_recv().is_err(), "only two records should remain");
    }

    #[test]
    fn scrollback_warn_flood_does_not_evict_errors() {
        let mut scrollback = Scrollback::new(4);
        scrollback.push(LogRecord {
            level: Level::Error,
            target: "t".into(),
            message: "the one error".into(),
            timestamp: Local::now(),
        });
        for i in 0..100 {
            scrollback.push(LogRecord {
                level: Level::Warn,
                target: "t".into(),
                message: format!("warn {i}"),
                timestamp: Local::now(),
            });
        }

        let errors: Vec<_> = scrollback.level(Level::Error).collect();
        assert_eq!(errors.len(), 1, "error survives the warn flood");
        assert_eq!(errors[0].message, "the one error");

        let warns: Vec<_> = scrollback.level(Level::Warn).collect();
        assert_eq!(warns.len(), 4, "warn ring is capped at per-severity cap");
        // Oldest warns evicted: newest four remain.
        assert_eq!(warns[0].message, "warn 96");
        assert_eq!(warns[3].message, "warn 99");
    }

    #[test]
    fn ordered_merges_severities_chronologically() {
        let t = |h, m, s| Local.with_ymd_and_hms(2026, 5, 31, h, m, s).unwrap();
        let mut sb = Scrollback::new(16);
        // Pushed out of time order across severities (as separate rings would hold them).
        sb.push(rec(Level::Warn, "warn :21", t(16, 55, 21)));
        sb.push(rec(Level::Warn, "warn :39", t(16, 56, 39)));
        sb.push(rec(Level::Info, "info :16", t(16, 55, 16)));
        sb.push(rec(Level::Error, "error :30", t(16, 55, 30)));

        let order: Vec<&str> = sb
            .ordered(LevelFilter::Info)
            .map(|r| r.message.as_str())
            .collect();
        assert_eq!(
            order,
            ["info :16", "warn :21", "error :30", "warn :39"],
            "records merge into one chronological stream across the per-severity rings"
        );

        // The display filter still applies; survivors stay time-ordered.
        let warn_and_up: Vec<&str> = sb
            .ordered(LevelFilter::Warn)
            .map(|r| r.message.as_str())
            .collect();
        assert_eq!(
            warn_and_up,
            ["warn :21", "error :30", "warn :39"],
            "Info excluded at the Warn filter; remainder still chronological"
        );
    }

    #[test]
    fn alert_is_sticky_until_viewed() {
        let alert = Arc::new(LogAlert::new(noop_repaint()));
        let scrollback = Arc::new(Mutex::new(Scrollback::new(16)));
        let viewing = Arc::new(AtomicBool::new(false));

        // An error arrives before the operator looks.
        alert.counts.store(AlertTally {
            errors: 1,
            warns: 0,
        });

        let mut state = LogStatusState::new(alert.clone(), scrollback.clone(), viewing.clone());
        // Constructed after the error, so acked already includes it: calm.
        assert!(matches!(state.alert(), Alert::Calm));

        // A fresh error after construction lights red and stays red until viewed.
        alert.counts.store(AlertTally {
            errors: 2,
            warns: 0,
        });
        assert!(matches!(state.alert(), Alert::Error));
        assert!(matches!(state.alert(), Alert::Error), "sticky until viewed");

        // Render the panel (marks viewed) → calm.
        {
            use egui_kittest::Harness;
            let mut harness = Harness::new_ui(|ui| {
                LogStatusPanel { state: &mut state }.ui(ui);
            });
            harness.run();
        }
        assert!(matches!(state.alert(), Alert::Calm), "viewing clears alert");

        // A later warning lights yellow.
        alert.counts.store(AlertTally {
            errors: 2,
            warns: 1,
        });
        assert!(matches!(state.alert(), Alert::Warn));
    }

    #[test]
    fn alert_counts_round_trips_alert_tally_across_u32_boundary() {
        let cell: NotifiedAtomic<AlertCounts> =
            NotifiedAtomic::new(AlertTally::default(), noop_repaint());

        let cases = [
            AlertTally {
                errors: 0,
                warns: 0,
            },
            AlertTally {
                errors: 1,
                warns: 7,
            },
            AlertTally {
                errors: u32::MAX,
                warns: 0,
            },
            AlertTally {
                errors: 0,
                warns: u32::MAX,
            },
            AlertTally {
                errors: u32::MAX,
                warns: u32::MAX,
            },
            AlertTally {
                errors: 0xDEAD_BEEF,
                warns: 0x0BAD_F00D,
            },
        ];
        for case in cases {
            cell.store(case);
            let got = cell.load();
            assert_eq!(got.errors, case.errors, "errors field round-trips");
            assert_eq!(got.warns, case.warns, "warns field round-trips");
        }
    }

    #[test]
    fn drain_thread_appends_then_wakes_on_new_alert() {
        let (signal, count) = counting_repaint();
        let alert = Arc::new(LogAlert::new(signal));
        let scrollback = Arc::new(Mutex::new(Scrollback::new(16)));
        let viewing = Arc::new(AtomicBool::new(false));

        let (logger, rx) = channel(16);
        let handle = spawn_drain_thread(rx, scrollback.clone(), alert.clone(), viewing.clone());

        emit(&logger, Level::Error, "boom");
        // Dropping the producer closes the channel so the drain thread exits.
        drop(logger);
        handle.join().unwrap();

        assert_eq!(alert.tally().errors, 1);
        assert!(
            count.load(Ordering::Relaxed) >= 1,
            "new error wakes the GUI"
        );

        let scrollback = scrollback.lock().unwrap();
        let errors: Vec<_> = scrollback.level(Level::Error).collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "boom");
    }

    /// A panel state pre-populated with a realistic mix of severities and a
    /// display filter — the fixture for the visual snapshots below.
    fn populated_panel_state(min_level: LevelFilter) -> LogStatusState {
        let scrollback = Arc::new(Mutex::new(Scrollback::new(64)));
        {
            let mut sb = scrollback.lock().unwrap();
            // The app captures only Warn and Error, so the fixture holds only those.
            // Fixed local timestamps keep the pixel snapshots deterministic.
            for (level, target, message, ts) in [
                (
                    Level::Error,
                    "dmx",
                    "universe 1 write failed: timed out",
                    Local.with_ymd_and_hms(2026, 5, 31, 21, 14, 7).unwrap(),
                ),
                (
                    Level::Warn,
                    "midi",
                    "APC40 reconnect attempt 2/5",
                    Local.with_ymd_and_hms(2026, 5, 31, 21, 14, 9).unwrap(),
                ),
            ] {
                sb.push(LogRecord {
                    level,
                    target: target.into(),
                    message: message.into(),
                    timestamp: ts,
                });
            }
        }
        let mut state = LogStatusState::new(
            Arc::new(LogAlert::new(noop_repaint())),
            scrollback,
            Arc::new(AtomicBool::new(true)),
        );
        state.min_level = min_level;
        state
    }

    #[test]
    fn snapshot_panel_warn_filter() {
        use egui_kittest::Harness;
        // Warn filter (the default): both the error (red) and warn (amber) lines show.
        let mut state = populated_panel_state(LevelFilter::Warn);
        let mut harness = Harness::new_ui(|ui| {
            LogStatusPanel { state: &mut state }.ui(ui);
        });
        harness.run();
        harness.snapshot("log_status_panel_warn_filter");
    }

    #[test]
    fn snapshot_panel_error_filter() {
        use egui_kittest::Harness;
        // Error filter: only the error line shows; the warn line is hidden.
        let mut state = populated_panel_state(LevelFilter::Error);
        let mut harness = Harness::new_ui(|ui| {
            LogStatusPanel { state: &mut state }.ui(ui);
        });
        harness.run();
        harness.snapshot("log_status_panel_error_filter");
    }

    #[test]
    fn snapshot_tab_label_states() {
        use egui_kittest::Harness;
        // Three independent states rendered as labels so the tab icon's color is
        // captured: calm (plain), warn (yellow ⏺), error (red ⏺).
        let calm = LogStatusState::new(
            Arc::new(LogAlert::new(noop_repaint())),
            Arc::new(Mutex::new(Scrollback::new(1))),
            Arc::new(AtomicBool::new(false)),
        );
        let warn_alert = Arc::new(LogAlert::new(noop_repaint()));
        let warn = LogStatusState::new(
            warn_alert.clone(),
            Arc::new(Mutex::new(Scrollback::new(1))),
            Arc::new(AtomicBool::new(false)),
        );
        warn_alert.counts.store(AlertTally {
            errors: 0,
            warns: 1,
        });
        let error_alert = Arc::new(LogAlert::new(noop_repaint()));
        let error = LogStatusState::new(
            error_alert.clone(),
            Arc::new(Mutex::new(Scrollback::new(1))),
            Arc::new(AtomicBool::new(false)),
        );
        error_alert.counts.store(AlertTally {
            errors: 1,
            warns: 0,
        });

        let mut harness = Harness::new_ui(|ui| {
            ui.label(status_tab_label(&calm));
            ui.separator();
            ui.label(status_tab_label(&warn));
            ui.separator();
            ui.label(status_tab_label(&error));
        });
        harness.run();
        harness.snapshot("log_status_tab_label");
    }

    /// Drives one record through the real drain thread and reports both the
    /// resulting wake count and the scrollback. Joins the thread, so it is fully
    /// deterministic (no sleeps).
    fn drain_one(
        level: Level,
        message: &str,
        viewing: bool,
    ) -> (usize, Arc<Mutex<Scrollback>>, Arc<LogAlert>) {
        let (signal, count) = counting_repaint();
        let alert = Arc::new(LogAlert::new(signal));
        let scrollback = Arc::new(Mutex::new(Scrollback::new(16)));
        let viewing = Arc::new(AtomicBool::new(viewing));

        let (logger, rx) = channel(16);
        let handle = spawn_drain_thread(rx, scrollback.clone(), alert.clone(), viewing);

        emit(&logger, level, message);
        // Closing the producer disconnects the channel so the drain thread exits.
        drop(logger);
        handle.join().unwrap();

        (count.load(Ordering::Relaxed), scrollback, alert)
    }

    #[test]
    fn info_wakes_gui_only_while_viewing() {
        // Tab closed: a plain INFO is recorded but must NOT wake the GUI (the
        // tally is unchanged and `viewing` is false).
        let (wakes, scrollback, alert) = drain_one(Level::Info, "fyi", false);
        assert_eq!(
            wakes, 0,
            "info must not wake the GUI when the Status tab is closed"
        );
        assert_eq!(alert.tally().errors, 0);
        assert_eq!(alert.tally().warns, 0);
        assert_eq!(
            scrollback.lock().unwrap().level(Level::Info).count(),
            1,
            "info is still recorded to scrollback even without a wake"
        );

        // Tab open: the same INFO wakes the GUI so the live list refreshes.
        let (wakes, _scrollback, _alert) = drain_one(Level::Info, "fyi", true);
        assert!(
            wakes >= 1,
            "info wakes the GUI while the Status tab is open"
        );

        // A warn always wakes, regardless of viewing (the tally changed).
        let (wakes, _scrollback, alert) = drain_one(Level::Warn, "heads up", false);
        assert!(
            wakes >= 1,
            "a new warn wakes the GUI even when the tab is closed"
        );
        assert_eq!(alert.tally().warns, 1);
    }

    #[test]
    fn status_tab_label_reflects_alert_state() {
        let alert = Arc::new(LogAlert::new(noop_repaint()));
        let scrollback = Arc::new(Mutex::new(Scrollback::new(16)));
        let viewing = Arc::new(AtomicBool::new(false));
        let mut state = LogStatusState::new(alert.clone(), scrollback, viewing);

        // Calm: the plain label, no alert glyph.
        assert_eq!(status_tab_label(&state).text(), "Status");

        // A warning lights the indicator (glyph appears).
        alert.counts.store(AlertTally {
            errors: 0,
            warns: 1,
        });
        assert!(matches!(state.alert(), Alert::Warn));
        assert_eq!(status_tab_label(&state).text(), "⏺ Status");

        // An error supersedes the warning; still lit.
        alert.counts.store(AlertTally {
            errors: 1,
            warns: 1,
        });
        assert!(matches!(state.alert(), Alert::Error));
        assert_eq!(status_tab_label(&state).text(), "⏺ Status");

        // Acknowledging (viewing) returns to the plain label.
        state.mark_viewed();
        assert!(matches!(state.alert(), Alert::Calm));
        assert_eq!(status_tab_label(&state).text(), "Status");
    }
}
