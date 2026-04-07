use anyhow::Result;
use eframe::egui;

use tunnels_audio::processor::ProcessorSettings;

fn init_logger() {
    simplelog::SimpleLogger::init(simplelog::LevelFilter::Info, simplelog::Config::default())
        .expect("failed to initialize logger");
}

fn main() -> Result<()> {
    init_logger();

    let processor_settings = ProcessorSettings::default();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_title("Audio Visualizer"),
        ..Default::default()
    };

    eframe::run_native(
        "Audio Visualizer",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(audio_vis::AudioVisApp::new(processor_settings)))
        }),
    )
    .unwrap();

    Ok(())
}
