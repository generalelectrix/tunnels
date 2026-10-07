//! Pinned per-buffer outputs: several channels of values in [0, 1], quantized
//! to 8 bits and compared against a checked-in golden. A difference shows up
//! as a per-channel report of how far, where, and how often the output moved,
//! and the actual output is written beside the golden.
//!
//! With `UPDATE_GOLDENS` set, the goldens are rewritten from the current
//! output instead.

use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Largest step allowed between golden and actual, in 8-bit units. Two steps
/// is 0.8 % of full scale: room for libm differences between platforms, far
/// below any behavioural change.
const TOLERANCE: u8 = 2;

/// One case's outputs, quantized to 8 bits, one row per `stride` buffers.
#[derive(Serialize, Deserialize)]
pub struct Golden {
    pub update_rate: f32,
    pub stride: usize,
    /// `channels[channel][row]`.
    pub channels: Vec<Vec<u8>>,
}

impl Golden {
    /// An empty record of `channels` channels.
    pub fn new(update_rate: f32, stride: usize, channels: usize) -> Self {
        Self {
            update_rate,
            stride,
            channels: vec![Vec::new(); channels],
        }
    }

    /// Record one row, one value per channel.
    pub fn push(&mut self, row: impl IntoIterator<Item = f32>) {
        for (channel, v) in self.channels.iter_mut().zip(row) {
            channel.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/golden")
}

/// Compare one case against its golden in `<name>.<extension>`, returning a
/// report if any channel differs beyond tolerance. `label` names a channel
/// in the report. Writes the actual output beside the golden when it differs.
pub fn check(
    name: &str,
    extension: &str,
    actual: &Golden,
    label: impl Fn(usize) -> String,
) -> Option<String> {
    let path = golden_dir().join(format!("{name}.{extension}"));
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::write(&path, postcard::to_allocvec(actual).expect("serialize"))
            .expect("write golden");
        return None;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}; run with UPDATE_GOLDENS=1",
            path.display()
        )
    });
    let golden: Golden = postcard::from_bytes(&bytes).expect("decode golden");
    assert_eq!(golden.stride, actual.stride, "{name}: stride changed");
    assert_eq!(
        golden.update_rate, actual.update_rate,
        "{name}: update rate changed"
    );
    assert_eq!(
        golden.channels.len(),
        actual.channels.len(),
        "{name}: the number of channels changed"
    );

    let secs_per_row = actual.stride as f32 / actual.update_rate;
    let mut report = String::new();
    for (channel, (g, a)) in golden.channels.iter().zip(&actual.channels).enumerate() {
        let label = label(channel);
        if g.len() != a.len() {
            let _ = writeln!(
                report,
                "  {label}: {} rows in golden, {} actual",
                g.len(),
                a.len()
            );
            continue;
        }
        let mut worst = 0u8;
        let mut worst_at = 0usize;
        let mut over = 0usize;
        for (i, (x, y)) in g.iter().zip(a).enumerate() {
            let d = x.abs_diff(*y);
            if d > worst {
                worst = d;
                worst_at = i;
            }
            if d > TOLERANCE {
                over += 1;
            }
        }
        if over > 0 {
            let _ = writeln!(
                report,
                "  {label}: {over} of {} rows differ by more than {TOLERANCE}/255; worst {worst}/255 at {:.2}s (golden {}, actual {})",
                g.len(),
                worst_at as f32 * secs_per_row,
                g[worst_at],
                a[worst_at]
            );
        }
    }
    if report.is_empty() {
        return None;
    }
    let actual_path = golden_dir().join(format!("{name}.actual.{extension}"));
    std::fs::write(
        &actual_path,
        postcard::to_allocvec(actual).expect("serialize"),
    )
    .expect("write actual");
    Some(format!(
        "{name} (actual written to {}):\n{report}",
        actual_path.display()
    ))
}
