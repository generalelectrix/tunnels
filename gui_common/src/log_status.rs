//! In-GUI logging capture and rolled-up status indicator.
//!
//! Captures `log` records on a lock-free, non-blocking producer path that never
//! blocks or locks, so it is safe to call from a real-time context. Retains a
//! finite per-severity scrollback and derives a sticky-until-viewed alert that
//! requests a repaint the instant a warn/error is captured.
//!
//! Three roles, two boundaries:
//! - Producer (`CaptureLogger`) → drain thread: a bounded `std::sync::mpsc`
//!   channel; `try_send` drops the record when full rather than blocking.
//! - Drain thread → consumer: a `NotifiedAtomic<AlertCounts>` whose `store`
//!   fires the `RepaintSignal` with no per-update heap allocation.

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

/// One captured log line. `timestamp` is the local wall-clock time at which the
/// record was observed.
pub struct LogRecord {
    pub level: Level,
    pub target: String,
    pub message: String,
    pub timestamp: DateTime<Local>,
}

/// Monotonic per-severity counts of captured records. Comparing a current
/// snapshot against a remembered one yields the sticky alert state.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct AlertTally {
    pub errors: u32,
    pub warns: u32,
}

/// Atomic cell holding an `AlertTally` packed as `(errors << 32) | warns` in a
/// single `AtomicU64`. The packing is private; callers see only `AlertTally`.
///
/// Packing both counts in one `u64` means a single atomic load returns errors
/// and warns as a matched, tear-free pair.
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

/// Shared alert surface: monotonic per-severity counts written by the drain
/// thread. A store fires the `RepaintSignal`; reads do not.
pub struct LogAlert {
    counts: NotifiedAtomic<AlertCounts>,
}

impl LogAlert {
    pub fn new(repaint: RepaintSignal) -> Self {
        Self {
            counts: NotifiedAtomic::new(AlertTally::default(), repaint),
        }
    }

    /// Current per-severity tally. Reading does not fire the `RepaintSignal`.
    pub fn tally(&self) -> AlertTally {
        self.counts.load()
    }
}

/// Producer-side `log` sink. Captured records are pushed to the drain thread
/// over a bounded channel; the path never blocks or locks, so it is safe to call
/// from a real-time context.
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
            // Pre-sized to the cap so the rings never reallocate during warm-up.
            by_level: std::array::from_fn(|_| VecDeque::with_capacity(per_severity_cap)),
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

    /// Remove every retained record at all severities.
    pub fn clear(&mut self) {
        for ring in &mut self.by_level {
            ring.clear();
        }
    }

    /// All retained records at or above `min_level`, merged across the
    /// per-severity rings **newest first** (so the most recent message renders at
    /// the top of the panel). Each ring is appended in arrival order, so iterating
    /// it back-to-front is newest-first; this is a lazy descending k-way merge — a
    /// heap of at most one head per ring, no full-list allocation or per-render
    /// sort. An old record of a rare severity still interleaves into its correct
    /// time position among newer frequent ones.
    pub fn newest_first(&self, min_level: LevelFilter) -> impl Iterator<Item = &LogRecord> {
        itertools::kmerge_by(
            self.by_level.iter().map(|ring| ring.iter().rev()),
            |a: &&LogRecord, b: &&LogRecord| a.timestamp >= b.timestamp,
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
/// `Warn`/`Error` carry the count of unacknowledged records at that severity.
pub enum Alert {
    Calm,
    Warn(u32),
    Error(u32),
}

/// Holds the shared alert and scrollback handles, the `acked` snapshot that
/// implements sticky-until-viewed, the `min_level` filter applied when listing
/// retained records, and `capture_level`, which mirrors the global log floor.
/// `min_level` and `capture_level` default to `Warn`.
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
    /// at the current tally so a fresh state opens Calm. `min_level` and
    /// `capture_level` both initialize to `Warn`.
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

    /// Record whether the live log view is currently in front.
    pub fn set_viewing(&self, active: bool) {
        self.viewing.store(active, Ordering::Relaxed);
    }

    /// Current rolled-up alert: `Error` with the unacknowledged error count if any
    /// new errors since last viewed, else `Warn` with the unacknowledged warn count
    /// if any new warnings, else `Calm`.
    pub fn alert(&self) -> Alert {
        let tally = self.alert.tally();
        let unacked_errors = tally.errors.saturating_sub(self.acked.errors);
        let unacked_warns = tally.warns.saturating_sub(self.acked.warns);
        if unacked_errors > 0 {
            Alert::Error(unacked_errors)
        } else if unacked_warns > 0 {
            Alert::Warn(unacked_warns)
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

        let mut clear_clicked = false;
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

            // "Clear" pins to the right edge of the row.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clear_clicked = ui.button("Clear").clicked();
            });
        });

        if clear_clicked {
            self.state.scrollback.lock().unwrap().clear();
        }

        ui.separator();

        let scrollback = self.state.scrollback.lock().unwrap();
        // Collect the newest-first merge once (cheap — references, already ordered),
        // then virtualize: `show_rows` only invokes the closure for the visible row
        // range, so layout cost is bounded by the viewport, not the whole buffer.
        // Rows are single-line monospace (uniform height, required by `show_rows`);
        // long lines are truncated rather than wrapped.
        let rows: Vec<&LogRecord> = scrollback.newest_first(self.state.min_level).collect();
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_height, rows.len(), |ui, row_range| {
                for record in row_range.filter_map(|i| rows.get(i).copied()) {
                    let color = match record.level {
                        Level::Error => STATUS_COLORS.error,
                        Level::Warn => STATUS_COLORS.warning,
                        _ => ui.style().visuals.text_color(),
                    };
                    let text = format!(
                        "{} [{}] {}: {}",
                        record.timestamp.format("%H:%M:%S"),
                        record.level,
                        record.target,
                        record.message
                    );
                    ui.add(
                        egui::Label::new(RichText::new(text).monospace().color(color))
                            .wrap_mode(egui::TextWrapMode::Truncate),
                    );
                }
            });
    }
}

/// Render the Status tab in a tab bar, returning `true` if it was clicked this
/// frame. When calm it renders as an ordinary selectable tab labelled "Status".
/// When an unviewed warn/error is pending the whole tab becomes a solid-colored
/// button — red `Error: <count>` for errors, amber `Warn: <count>` for warnings —
/// in dark text, where `<count>` is the number of unacknowledged records at that
/// severity.
pub fn status_tab(ui: &mut egui::Ui, selected: bool, state: &LogStatusState) -> bool {
    let (text, color) = match state.alert() {
        Alert::Calm => return ui.selectable_label(selected, "Status").clicked(),
        Alert::Warn(count) => (format!("Warn: {count}"), STATUS_COLORS.warning),
        Alert::Error(count) => (format!("Error: {count}"), STATUS_COLORS.error),
    };
    // `egui::Button` paints its fill unconditionally (unlike a selectable, which
    // only fills when selected/hovered), so the whole tab shows the alert color.
    let label = RichText::new(text).color(egui::Color32::from_gray(20));
    ui.add(egui::Button::new(label).fill(color)).clicked()
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
    fn clear_empties_all_severities() {
        let now = Local::now();
        let mut sb = Scrollback::new(16);
        sb.push(rec(Level::Error, "err", now));
        sb.push(rec(Level::Warn, "warn", now));
        sb.push(rec(Level::Info, "info", now));
        assert_eq!(sb.newest_first(LevelFilter::Trace).count(), 3);

        sb.clear();

        assert_eq!(
            sb.newest_first(LevelFilter::Trace).count(),
            0,
            "clear empties every severity"
        );
        assert_eq!(sb.level(Level::Error).count(), 0);
        assert_eq!(sb.level(Level::Warn).count(), 0);
        assert_eq!(sb.level(Level::Info).count(), 0);
    }

    #[test]
    fn newest_first_merges_across_severities() {
        let t = |h, m, s| Local.with_ymd_and_hms(2026, 5, 31, h, m, s).unwrap();
        let mut sb = Scrollback::new(16);
        // Pushed out of time order across severities (as separate rings would hold them).
        sb.push(rec(Level::Warn, "warn :21", t(16, 55, 21)));
        sb.push(rec(Level::Warn, "warn :39", t(16, 56, 39)));
        sb.push(rec(Level::Info, "info :16", t(16, 55, 16)));
        sb.push(rec(Level::Error, "error :30", t(16, 55, 30)));

        // Newest first, so the most recent message renders at the top of the panel.
        let order: Vec<&str> = sb
            .newest_first(LevelFilter::Info)
            .map(|r| r.message.as_str())
            .collect();
        assert_eq!(
            order,
            ["warn :39", "error :30", "warn :21", "info :16"],
            "records merge newest-first across the per-severity rings"
        );

        // The display filter still applies; survivors stay newest-first.
        let warn_and_up: Vec<&str> = sb
            .newest_first(LevelFilter::Warn)
            .map(|r| r.message.as_str())
            .collect();
        assert_eq!(
            warn_and_up,
            ["warn :39", "error :30", "warn :21"],
            "Info excluded at the Warn filter; remainder still newest-first"
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

        // Two more errors after construction light red, reporting the unacked count
        // (3 total - 1 acked = 2), and stay lit until viewed.
        alert.counts.store(AlertTally {
            errors: 3,
            warns: 0,
        });
        assert!(matches!(state.alert(), Alert::Error(2)));
        assert!(
            matches!(state.alert(), Alert::Error(2)),
            "sticky until viewed"
        );

        // Render the panel (marks viewed) → calm.
        {
            use egui_kittest::Harness;
            let mut harness = Harness::new_ui(|ui| {
                LogStatusPanel { state: &mut state }.ui(ui);
            });
            harness.run();
        }
        assert!(matches!(state.alert(), Alert::Calm), "viewing clears alert");

        // A later warning lights yellow with its own unacked count.
        alert.counts.store(AlertTally {
            errors: 3,
            warns: 1,
        });
        assert!(matches!(state.alert(), Alert::Warn(1)));
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
        // Three independent states rendered as tabs in a row: calm (plain tab),
        // warn (amber "Status N" button), error (red "Status N" button).
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
            warns: 3,
        });
        let error_alert = Arc::new(LogAlert::new(noop_repaint()));
        let error = LogStatusState::new(
            error_alert.clone(),
            Arc::new(Mutex::new(Scrollback::new(1))),
            Arc::new(AtomicBool::new(false)),
        );
        error_alert.counts.store(AlertTally {
            errors: 2,
            warns: 0,
        });

        let mut harness = Harness::new_ui(|ui| {
            ui.horizontal(|ui| {
                let _ = status_tab(ui, false, &calm);
                let _ = status_tab(ui, false, &warn);
                let _ = status_tab(ui, false, &error);
            });
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
    fn alert_reports_severity_and_unacked_count() {
        let alert = Arc::new(LogAlert::new(noop_repaint()));
        let scrollback = Arc::new(Mutex::new(Scrollback::new(16)));
        let viewing = Arc::new(AtomicBool::new(false));
        let mut state = LogStatusState::new(alert.clone(), scrollback, viewing);

        // Fresh state: calm.
        assert!(matches!(state.alert(), Alert::Calm));

        // Warnings light amber and report their count.
        alert.counts.store(AlertTally {
            errors: 0,
            warns: 4,
        });
        assert!(matches!(state.alert(), Alert::Warn(4)));

        // Any unacked error supersedes warnings, reporting the error count.
        alert.counts.store(AlertTally {
            errors: 2,
            warns: 4,
        });
        assert!(matches!(state.alert(), Alert::Error(2)));

        // Acknowledging (viewing) clears the alert.
        state.mark_viewed();
        assert!(matches!(state.alert(), Alert::Calm));
    }
}
