use anyhow::Result;
use eframe::egui;

use tunnels::audio::processor::ProcessorSettings;
use tunnels::audio::reconnect::ReconnectingInput;

fn init_logger() {
    simplelog::SimpleLogger::init(simplelog::LevelFilter::Info, simplelog::Config::default())
        .expect("failed to initialize logger");
}

fn main() -> Result<()> {
    init_logger();

    let device_name = tunnels::audio::prompt_audio()?;
    let processor_settings = ProcessorSettings::default();

    // Keep the input alive for the lifetime of the app.
    let _input = device_name.map(|name| {
        ReconnectingInput::new(name, processor_settings.clone())
    });

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
