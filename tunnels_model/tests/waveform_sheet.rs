//! Contact sheets of the waveforms, as checked-in goldens.
//!
//! One sheet per waveform, laid out so that every parameter that changes the
//! shape of one period is on it at once.
//!
//! Smoothing increases down the rows: 0, 1/4, 1/2, 3/4, 1. Each row runs
//! across six columns — duty cycle 1, 1/2 and 1/4 unpulsed, then the same
//! three pulsed. Every cell draws one period over a fixed bipolar range, so a
//! pulse's rest sits on the same line a bipolar wave crosses, and a frame
//! marks where the range ends.
//!
//! Nothing is labelled and nothing is antialiased: a label needs a font and a
//! blend needs a rounding, and either one puts differences into a golden that
//! are not differences in a waveform. The layout is documented here instead of
//! drawn onto the image.

use std::path::Path;

use golden_image::{Goldens, Tolerance};
use image::{Rgba, RgbaImage};
use tunnels_lib::number::{Phase, UnipolarFloat};
use tunnels_model::waveforms::{WaveformArgs, sine_square, tri_saw};

/// A waveform as a sheet samples it: one number for one phase.
type Waveform = fn(&WaveformArgs) -> f64;

/// The waveforms a sheet is drawn for, each named after the golden it is
/// compared against.
const WAVEFORMS: [(&str, Waveform); 2] =
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

/// The row inside a cell that a value is drawn on, over a fixed bipolar range
/// whose ends are the rows just inside the frame.
fn row_of(value: f64) -> u32 {
    let span = f64::from(TRACE_HEIGHT - 1);
    (((1.0 - value.clamp(-1.0, 1.0)) / 2.0) * span).round() as u32
}

/// One period of `waveform` at the given settings, sampled once per pixel
/// column of a cell.
fn trace_rows(waveform: Waveform, smoothing: f64, duty_cycle: f64, pulse: bool) -> Vec<u32> {
    (0..TRACE_WIDTH)
        .map(|x| {
            row_of(waveform(&WaveformArgs {
                phase_spatial: Phase::new(f64::from(x) / f64::from(TRACE_WIDTH)),
                // A sheet is the shape of one period, not its travel. At rest
                // a standing wave is at full amplitude and a travelling one
                // has no offset, so the two draw the same cell and only the
                // shape is left.
                phase_temporal: Phase::ZERO,
                standing: false,
                smoothing: UnipolarFloat::new(smoothing),
                duty_cycle: UnipolarFloat::new(duty_cycle),
                pulse,
            }))
        })
        .collect()
}

/// Draw one framed cell into `sheet` at the given cell row and column.
fn draw_cell(
    sheet: &mut RgbaImage,
    (row, column): (u32, u32),
    waveform: Waveform,
    smoothing: f64,
    duty_cycle: f64,
    pulse: bool,
) {
    let (x0, y0) = (column * CELL_WIDTH, row * CELL_HEIGHT);

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

    // The vertical span between consecutive samples is filled, so a hard edge
    // stays a connected line rather than a sample at either end of it.
    let rows = trace_rows(waveform, smoothing, duty_cycle, pulse);
    for (x, &here) in rows.iter().enumerate() {
        let next = rows.get(x + 1).copied().unwrap_or(here);
        for y in here.min(next)..=here.max(next) {
            sheet.put_pixel(x0 + 1 + x as u32, y0 + 1 + y, TRACE);
        }
    }
}

/// A sheet of `waveform` at every smoothing, duty cycle and pulse setting.
fn contact_sheet(waveform: Waveform) -> RgbaImage {
    let settings: Vec<(f64, bool)> = [false, true]
        .into_iter()
        .flat_map(|pulse| DUTY_CYCLES.map(|duty_cycle| (duty_cycle, pulse)))
        .collect();

    let mut sheet = RgbaImage::new(
        settings.len() as u32 * CELL_WIDTH,
        SMOOTHINGS.len() as u32 * CELL_HEIGHT,
    );
    for (row, &smoothing) in SMOOTHINGS.iter().enumerate() {
        for (column, &(duty_cycle, pulse)) in settings.iter().enumerate() {
            draw_cell(
                &mut sheet,
                (row as u32, column as u32),
                waveform,
                smoothing,
                duty_cycle,
                pulse,
            );
        }
    }
    sheet
}

/// Every waveform draws the sheet its golden holds.
///
/// A sheet is drawn by integer arithmetic over a fixed set of parameters, so
/// any difference at all is a difference in a waveform.
#[test]
fn waveforms_draw_their_sheets() {
    let goldens = Goldens::new(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("image_mismatch")
            .join(env!("CARGO_PKG_NAME")),
        Tolerance::EXACT,
    );

    for (name, waveform) in WAVEFORMS {
        goldens.compare(&contact_sheet(waveform), name);
    }
}
