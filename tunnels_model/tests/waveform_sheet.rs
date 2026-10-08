//! Contact sheets of the waveforms, as checked-in goldens.
//!
//! One sheet per waveform, laid out so that every parameter that changes the
//! shape of one period is on it at once, and identically across the sheets so
//! that three of them can be read side by side.
//!
//! Smoothing increases down the rows: 0, 1/4, 1/2, 3/4, 1. Each row runs
//! across six columns — duty cycle 1, 1/2 and 1/4 unpulsed, then the same
//! three pulsed. Every cell draws one period over a fixed bipolar range, so a
//! pulse's rest sits on the same line a bipolar wave crosses, and a frame
//! marks where the range ends.
//!
//! The noise sheet draws four traces to a cell rather than one, at offset
//! indices 0 to 3, brightest trace first and each one after it dimmer. Noise
//! takes smoothing as a cross-correlation term rather than as a shape, so what
//! a cell shows is how far apart those four offsets are: at smoothing 1 they
//! are one curve, and at smoothing 0 four unrelated ones. That spread closing
//! down the rows is what the sheet is for.
//!
//! Nothing is labelled and nothing is antialiased: a label needs a font and a
//! blend needs a rounding, and either one puts differences into a golden that
//! are not differences in a waveform. The layout is documented here instead of
//! drawn onto the image.

use std::path::Path;
use std::time::Duration;

use golden_image::{Goldens, Tolerance};
use image::{Rgba, RgbaImage};
use tunnels_lib::number::{Phase, UnipolarFloat};
use tunnels_model::animation::{
    Animation, ControlMessage, EmitStateChange, PreparedAnimation, StateChange, Waveform,
};
use tunnels_model::clock_bank::ClockBank;
use tunnels_model::waveforms::{WaveformArgs, sine_square, tri_saw};

/// A waveform as a sheet samples it: one number for one phase.
type WaveformFn = fn(&WaveformArgs) -> f64;

/// The waveforms drawn from a pure function of their arguments, each named
/// after the golden it is compared against.
const WAVEFORM_FNS: [(&str, WaveformFn); 2] =
    [("sine_square.png", sine_square), ("tri_saw.png", tri_saw)];

/// Smoothing down the rows.
const SMOOTHINGS: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// Duty cycle across the columns, run unpulsed and then pulsed.
const DUTY_CYCLES: [f64; 3] = [1.0, 0.5, 0.25];

/// A cell's full size, frame included.
///
/// The height is odd so that the zero line falls on a pixel exactly halfway
/// between the ends of the range, and a wave and its negation are drawn the
/// same distance from it.
const CELL_WIDTH: u32 = 300;
const CELL_HEIGHT: u32 = 151;

/// The part of a cell the waveform is drawn into, inside the frame.
const TRACE_WIDTH: u32 = CELL_WIDTH - 2;
const TRACE_HEIGHT: u32 = CELL_HEIGHT - 2;

const BACKGROUND: Rgba<u8> = Rgba([16, 16, 16, 255]);
const FRAME: Rgba<u8> = Rgba([80, 80, 80, 255]);
const ZERO_LINE: Rgba<u8> = Rgba([64, 64, 64, 255]);
const TRACE: Rgba<u8> = Rgba([255, 255, 255, 255]);

/// The colours the noise offsets are drawn in, offset 0 first.
///
/// Every one of them is brighter than the frame, so that the dimmest trace
/// still reads as a trace rather than as part of the cell.
const OFFSET_TRACES: [Rgba<u8>; 4] = [
    Rgba([255, 255, 255, 255]),
    Rgba([180, 180, 180, 255]),
    Rgba([140, 140, 140, 255]),
    Rgba([105, 105, 105, 255]),
];

/// How many periods of noise a cell's phase sweep covers.
///
/// Noise varies over about a unit of its field, and a sweep covers this many
/// units, so this is directly how many features of noise a cell shows. Few
/// enough to read as a curve rather than as hash, and enough of them that the
/// trace reaches both ends of the gate rather than wandering across the
/// middle. It is also how many times the duty cycle opens and closes across a
/// cell, because the gate wraps the phase it is given.
const NOISE_PERIODS: u16 = 3;

/// Long enough for the smoothing smoother to arrive wherever it was sent.
const SETTLE: Duration = Duration::from_secs(1);

/// The row inside a cell that a value is drawn on, over a fixed bipolar range
/// whose ends are the rows just inside the frame.
fn row_of(value: f64) -> u32 {
    let span = f64::from(TRACE_HEIGHT - 1);
    (((1.0 - value.clamp(-1.0, 1.0)) / 2.0) * span).round() as u32
}

/// The rows a trace occupies, sampled once per pixel column of a cell.
fn trace_rows(sample: impl Fn(Phase) -> f64) -> Vec<u32> {
    (0..TRACE_WIDTH)
        .map(|x| row_of(sample(Phase::new(f64::from(x) / f64::from(TRACE_WIDTH)))))
        .collect()
}

/// Paint a cell's background, its frame, and its zero line.
fn draw_frame(sheet: &mut RgbaImage, (x0, y0): (u32, u32)) {
    for y in 0..CELL_HEIGHT {
        for x in 0..CELL_WIDTH {
            let on_frame = x == 0 || y == 0 || x == CELL_WIDTH - 1 || y == CELL_HEIGHT - 1;
            sheet.put_pixel(x0 + x, y0 + y, if on_frame { FRAME } else { BACKGROUND });
        }
    }

    let zero = row_of(0.0);
    for x in 0..TRACE_WIDTH {
        sheet.put_pixel(x0 + 1 + x, y0 + 1 + zero, ZERO_LINE);
    }
}

/// Draw one trace across a cell.
///
/// The vertical span between consecutive samples is filled, so a hard edge
/// stays a connected line rather than a sample at either end of it.
fn draw_trace(sheet: &mut RgbaImage, (x0, y0): (u32, u32), colour: Rgba<u8>, rows: &[u32]) {
    for (x, &here) in rows.iter().enumerate() {
        let next = rows.get(x + 1).copied().unwrap_or(here);
        for y in here.min(next)..=here.max(next) {
            sheet.put_pixel(x0 + 1 + x as u32, y0 + 1 + y, colour);
        }
    }
}

/// The settings each column of a sheet is drawn at, in order.
fn columns() -> Vec<(f64, bool)> {
    [false, true]
        .into_iter()
        .flat_map(|pulse| DUTY_CYCLES.map(|duty_cycle| (duty_cycle, pulse)))
        .collect()
}

/// A sheet sized for the grid, with every cell framed and `draw` run on each.
fn sheet_of(mut draw: impl FnMut(&mut RgbaImage, (u32, u32), f64, f64, bool)) -> RgbaImage {
    let columns = columns();
    let mut sheet = RgbaImage::new(
        columns.len() as u32 * CELL_WIDTH,
        SMOOTHINGS.len() as u32 * CELL_HEIGHT,
    );
    for (row, &smoothing) in SMOOTHINGS.iter().enumerate() {
        for (column, &(duty_cycle, pulse)) in columns.iter().enumerate() {
            let origin = (column as u32 * CELL_WIDTH, row as u32 * CELL_HEIGHT);
            draw_frame(&mut sheet, origin);
            draw(&mut sheet, origin, smoothing, duty_cycle, pulse);
        }
    }
    sheet
}

/// A sheet of `waveform` at every smoothing, duty cycle and pulse setting.
fn waveform_sheet(waveform: WaveformFn) -> RgbaImage {
    sheet_of(|sheet, origin, smoothing, duty_cycle, pulse| {
        let rows = trace_rows(|phase_spatial| {
            waveform(&WaveformArgs {
                phase_spatial,
                // A sheet is the shape of one period, not its travel. At rest
                // a standing wave is at full amplitude and a travelling one
                // has no offset, so the two draw the same cell and only the
                // shape is left.
                phase_temporal: Phase::ZERO,
                standing: false,
                smoothing: UnipolarFloat::new(smoothing),
                duty_cycle: UnipolarFloat::new(duty_cycle),
                pulse,
            })
        });
        draw_trace(sheet, origin, TRACE, &rows);
    })
}

/// Discards the state changes an animation reports while it is being set up.
struct Discard;

impl EmitStateChange for Discard {
    fn emit_animation_state_change(&mut self, _: StateChange) {}
}

/// An animation driving noise at the given settings, resolved for a frame.
///
/// Smoothing is reached over time rather than set, so it is sent and then
/// allowed to arrive. An animation at no size holds its clock still while that
/// happens, and the size it needs to run is given afterwards, which leaves the
/// animation's own elapsed time at zero and the field it reads fixed.
fn noise_animation(smoothing: f64, duty_cycle: f64, pulse: bool) -> PreparedAnimation {
    let mut animation = Animation::default();
    for change in [
        StateChange::Waveform(Waveform::Noise),
        StateChange::NPeriods(NOISE_PERIODS),
        StateChange::DutyCycle(UnipolarFloat::new(duty_cycle)),
        StateChange::Pulse(pulse),
        StateChange::Smoothing(UnipolarFloat::new(smoothing)),
    ] {
        animation.control(ControlMessage::Set(change), &mut Discard);
    }

    animation.update_state(SETTLE, UnipolarFloat::ZERO);
    animation.control(
        ControlMessage::Set(StateChange::Size(UnipolarFloat::ONE)),
        &mut Discard,
    );
    animation.prepare(&ClockBank::default(), UnipolarFloat::ZERO)
}

/// The rows each noise offset traces in one cell, offset 0 first.
fn noise_rows(smoothing: f64, duty_cycle: f64, pulse: bool) -> Vec<Vec<u32>> {
    let animation = noise_animation(smoothing, duty_cycle, pulse);
    (0..OFFSET_TRACES.len())
        .map(|offset| trace_rows(|phase| animation.unit_value(phase, offset)))
        .collect()
}

/// A sheet of noise at every smoothing, duty cycle and pulse setting.
fn noise_sheet() -> RgbaImage {
    sheet_of(|sheet, origin, smoothing, duty_cycle, pulse| {
        // Drawn from the dimmest offset up, so that where offsets coincide the
        // brightest is the one left showing.
        for (offset, rows) in noise_rows(smoothing, duty_cycle, pulse)
            .iter()
            .enumerate()
            .rev()
        {
            draw_trace(sheet, origin, OFFSET_TRACES[offset], rows);
        }
    })
}

/// This suite's goldens. A sheet is drawn by integer arithmetic over a fixed
/// set of parameters, so any difference at all is a difference in a waveform.
fn goldens() -> Goldens {
    Goldens::new(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("image_mismatch")
            .join(env!("CARGO_PKG_NAME")),
        Tolerance::EXACT,
    )
}

/// Every waveform draws the sheet its golden holds.
#[test]
fn waveforms_draw_their_sheets() {
    let goldens = goldens();
    for (name, waveform) in WAVEFORM_FNS {
        goldens.compare(&waveform_sheet(waveform), name);
    }
    goldens.compare(&noise_sheet(), "noise.png");
}

/// Smoothing is what decides how far apart noise holds the offsets it is read
/// at: at full it reads every offset off one trajectory, and at none it reads
/// each off a trajectory of its own.
///
/// This is the claim the noise sheet is drawn to show, and a sheet alone
/// cannot fail when it stops being true.
#[test]
fn smoothing_closes_the_spread_between_noise_offsets() {
    let spread = |smoothing: f64| {
        let rows = noise_rows(smoothing, 1.0, false);
        rows.iter().filter(|other| **other != rows[0]).count()
    };

    assert_eq!(
        spread(1.0),
        0,
        "at full smoothing the offsets traced different curves"
    );
    assert_eq!(
        spread(0.0),
        OFFSET_TRACES.len() - 1,
        "at no smoothing some offset traced the same curve as offset 0"
    );
}

/// A pulsed cell is drawn across enough of the noise field to show the gate
/// flat at both of its ends: some of the cell resting at dark, some of it
/// arrived at full.
///
/// Noise crosses the upper knee only rarely, so a sweep too narrow to reach it
/// would draw a sheet that shows a pulse rising and falling and never
/// saturating, which is the wrong picture of the gate.
#[test]
fn a_pulsed_cell_reaches_both_ends_of_the_gate() {
    let animation = noise_animation(1.0, 1.0, true);
    let values: Vec<f64> = (0..TRACE_WIDTH)
        .map(|x| animation.unit_value(Phase::new(f64::from(x) / f64::from(TRACE_WIDTH)), 0))
        .collect();

    let resting = values.iter().filter(|v| **v == 0.0).count();
    let full = values.iter().filter(|v| **v == 1.0).count();
    assert!(
        resting > 0 && full > 0,
        "across {TRACE_WIDTH} columns a pulsed cell rested at dark for {resting} of them \
         and reached full for {full}"
    );
}
